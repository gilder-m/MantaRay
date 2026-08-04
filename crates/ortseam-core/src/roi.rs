//! Regions of interest.
//!
//! A [`Roi`] is an inclusive channel range. A [`RoiSet`] keeps regions sorted
//! and non-overlapping; contiguous (touching but not overlapping) regions stay
//! separate so that close peaks are still reported individually, matching
//! MAESTRO's behaviour.

use std::ops::RangeInclusive;

use serde::{Deserialize, Serialize};

/// An inclusive region of interest.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Roi {
    /// First channel (inclusive).
    pub start: usize,
    /// Last channel (inclusive).
    pub end: usize,
}

impl Roi {
    /// Creates a region, ordering the bounds.
    pub fn new(a: usize, b: usize) -> Self {
        Self {
            start: a.min(b),
            end: a.max(b),
        }
    }

    /// Number of channels covered (always at least one).
    pub fn len(&self) -> usize {
        self.end - self.start + 1
    }

    /// Always false: a region covers at least one channel.
    pub fn is_empty(&self) -> bool {
        false
    }

    /// True when `channel` is inside the region.
    pub fn contains(&self, channel: usize) -> bool {
        channel >= self.start && channel <= self.end
    }

    /// True when the regions share at least one channel.
    pub fn overlaps(&self, other: &Roi) -> bool {
        self.start <= other.end && other.start <= self.end
    }

    /// True when the regions overlap or sit directly next to each other.
    pub fn touches(&self, other: &Roi) -> bool {
        self.start <= other.end.saturating_add(1) && other.start <= self.end.saturating_add(1)
    }

    /// Centre channel as a fraction.
    pub fn center(&self) -> f64 {
        (self.start + self.end) as f64 / 2.0
    }

    /// Iterator over the channels in the region.
    pub fn channels(&self) -> RangeInclusive<usize> {
        self.start..=self.end
    }

    /// Restricts the region to a spectrum of `length` channels.
    pub fn clamped(&self, length: usize) -> Option<Roi> {
        if length == 0 || self.start >= length {
            return None;
        }
        Some(Roi::new(self.start, self.end.min(length - 1)))
    }
}

/// The set of regions marked on a spectrum.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoiSet {
    rois: Vec<Roi>,
}

impl RoiSet {
    /// Empty set.
    pub fn new() -> Self {
        Self::default()
    }

    /// Builds a set from `(start, end)` pairs.
    pub fn from_pairs<I>(pairs: I) -> Self
    where
        I: IntoIterator<Item = (usize, usize)>,
    {
        let mut set = Self::default();
        for (a, b) in pairs {
            set.mark(Roi::new(a, b));
        }
        set
    }

    /// The regions as `(start, end)` pairs.
    pub fn to_pairs(&self) -> Vec<(usize, usize)> {
        self.rois.iter().map(|r| (r.start, r.end)).collect()
    }

    /// Marks a region, merging it with any regions it overlaps.
    pub fn mark(&mut self, roi: Roi) {
        let mut merged = roi;
        let mut kept: Vec<Roi> = Vec::with_capacity(self.rois.len() + 1);
        for existing in self.rois.drain(..) {
            if existing.overlaps(&merged) {
                merged = Roi::new(
                    merged.start.min(existing.start),
                    merged.end.max(existing.end),
                );
            } else {
                kept.push(existing);
            }
        }
        kept.push(merged);
        kept.sort();
        self.rois = kept;
    }

    /// Unmarks a channel range, trimming or splitting regions as needed.
    pub fn unmark(&mut self, range: Roi) {
        let mut out: Vec<Roi> = Vec::with_capacity(self.rois.len() + 1);
        for existing in self.rois.drain(..) {
            if !existing.overlaps(&range) {
                out.push(existing);
                continue;
            }
            if existing.start < range.start {
                out.push(Roi::new(existing.start, range.start - 1));
            }
            if existing.end > range.end {
                out.push(Roi::new(range.end + 1, existing.end));
            }
        }
        out.sort();
        self.rois = out;
    }

    /// Removes the region containing `channel` (ROI/Clear). Returns whether one
    /// was removed.
    pub fn clear_at(&mut self, channel: usize) -> bool {
        match self.index_at(channel) {
            Some(index) => {
                self.rois.remove(index);
                true
            }
            None => false,
        }
    }

    /// Removes every region (ROI/Clear All).
    pub fn clear_all(&mut self) {
        self.rois.clear();
    }

    /// The region containing `channel`.
    pub fn at(&self, channel: usize) -> Option<&Roi> {
        self.index_at(channel).map(|index| &self.rois[index])
    }

    /// Index of the region containing `channel`.
    pub fn index_at(&self, channel: usize) -> Option<usize> {
        // The set is sorted and non-overlapping, so at most one region can hold
        // a channel, and it is the last one starting at or before it. A display
        // asks this for every channel of every column, which is enough times
        // that walking the whole list starts to show.
        let after = self.rois.partition_point(|roi| roi.start <= channel);
        let candidate = after.checked_sub(1)?;
        self.rois[candidate].contains(channel).then_some(candidate)
    }

    /// Whether any region shares a channel with the inclusive range.
    ///
    /// One question instead of one per channel: drawing asks whether a column
    /// of the spectrum touches a region at all, and a column can be hundreds of
    /// channels wide.
    pub fn intersects(&self, start: usize, end: usize) -> bool {
        let (start, end) = (start.min(end), start.max(end));
        // The first region that could reach into the range is the last one
        // starting at or before its end; anything later starts beyond it.
        let after = self.rois.partition_point(|roi| roi.start <= end);
        let Some(candidate) = after.checked_sub(1) else {
            return false;
        };
        // Only that one needs testing: regions do not overlap, so if it ends
        // before the range begins, every earlier region does too.
        self.rois[candidate].end >= start
    }

    /// Region by index.
    pub fn get(&self, index: usize) -> Option<&Roi> {
        self.rois.get(index)
    }

    /// First region that starts after `channel` (the "next ROI" button).
    pub fn next_after(&self, channel: usize) -> Option<&Roi> {
        self.rois.iter().find(|r| r.start > channel)
    }

    /// Last region that ends before `channel` (the "previous ROI" button).
    pub fn previous_before(&self, channel: usize) -> Option<&Roi> {
        self.rois.iter().rev().find(|r| r.end < channel)
    }

    /// Iterates the regions in channel order.
    pub fn iter(&self) -> std::slice::Iter<'_, Roi> {
        self.rois.iter()
    }

    /// Number of regions.
    pub fn len(&self) -> usize {
        self.rois.len()
    }

    /// True when nothing is marked.
    pub fn is_empty(&self) -> bool {
        self.rois.is_empty()
    }

    /// Total number of channels covered by all regions.
    pub fn marked_channels(&self) -> usize {
        self.rois.iter().map(|r| r.len()).sum()
    }

    /// The regions as a slice.
    pub fn as_slice(&self) -> &[Roi] {
        &self.rois
    }

    /// Drops regions that fall outside a spectrum of `length` channels and
    /// clips those that straddle the end.
    pub fn clamp_to(&mut self, length: usize) {
        self.rois = self
            .rois
            .iter()
            .filter_map(|r| r.clamped(length))
            .collect::<Vec<_>>();
    }
}

impl<'a> IntoIterator for &'a RoiSet {
    type Item = &'a Roi;
    type IntoIter = std::slice::Iter<'a, Roi>;

    fn into_iter(self) -> Self::IntoIter {
        self.rois.iter()
    }
}

impl FromIterator<Roi> for RoiSet {
    fn from_iter<I: IntoIterator<Item = Roi>>(iter: I) -> Self {
        let mut set = Self::default();
        for roi in iter {
            set.mark(roi);
        }
        set
    }
}

#[cfg(test)]
mod ordering_tests {
    use super::*;

    /// Every way of building a set, checked for the invariant the lookups rely on.
    fn assert_ordered(set: &RoiSet, how: &str) {
        for pair in set.iter().collect::<Vec<_>>().windows(2) {
            assert!(
                pair[0].start <= pair[1].start,
                "{how}: regions out of order, {:?} before {:?}",
                pair[0],
                pair[1]
            );
            assert!(
                !pair[0].overlaps(pair[1]),
                "{how}: regions overlap, {:?} and {:?}",
                pair[0],
                pair[1]
            );
        }
    }

    #[test]
    fn a_set_stays_sorted_and_separate_however_it_is_built() {
        // The lookups binary-search on this, so it is worth asserting rather
        // than trusting the comment at the top of the file.
        let mut set = RoiSet::new();
        for (a, b) in [(90, 100), (10, 20), (50, 60), (95, 120), (0, 5), (55, 58)] {
            set.mark(Roi::new(a, b));
            assert_ordered(&set, "after marking");
        }
        set.unmark(Roi::new(12, 15));
        assert_ordered(&set, "after unmarking the middle of a region");
        set.unmark(Roi::new(0, 1000));
        assert!(set.is_empty(), "unmarking everything should leave nothing");
    }

    #[test]
    fn a_channel_finds_the_region_holding_it() {
        let set = RoiSet::from_pairs([(10, 20), (50, 60), (90, 120)]);
        for (channel, expected) in [
            (0, None),
            (9, None),
            (10, Some(Roi::new(10, 20))),
            (15, Some(Roi::new(10, 20))),
            (20, Some(Roi::new(10, 20))),
            (21, None),
            (49, None),
            (55, Some(Roi::new(50, 60))),
            (61, None),
            (120, Some(Roi::new(90, 120))),
            (121, None),
            (usize::MAX, None),
        ] {
            assert_eq!(
                set.at(channel).copied(),
                expected,
                "channel {channel} landed in the wrong region"
            );
        }
    }

    #[test]
    fn a_range_knows_whether_it_touches_any_region() {
        // What the display asks once per column, instead of asking about every
        // channel in the column separately.
        let set = RoiSet::from_pairs([(10, 20), (50, 60), (90, 120)]);
        for (start, end, expected) in [
            (0, 9, false),
            (0, 10, true),
            (20, 20, true),
            (21, 49, false),
            (21, 50, true),
            (61, 89, false),
            (0, 1000, true),
            (121, 1000, false),
        ] {
            assert_eq!(
                set.intersects(start, end),
                expected,
                "range {start}..={end} answered wrongly"
            );
        }
        assert!(
            !RoiSet::new().intersects(0, 1000),
            "an empty set touches nothing"
        );
    }
}
