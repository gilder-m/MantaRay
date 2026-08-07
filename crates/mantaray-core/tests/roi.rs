//! Region-of-interest bookkeeping (MAESTRO ROI menu, §4.5).

use mantaray_core::{Roi, RoiSet};

#[test]
fn roi_normalises_its_bounds() {
    let r = Roi::new(50, 10);
    assert_eq!(r.start, 10);
    assert_eq!(r.end, 50);
    assert_eq!(r.len(), 41, "bounds are inclusive");
    assert!(r.contains(10) && r.contains(50) && !r.contains(51));
}

#[test]
fn marking_merges_overlapping_regions() {
    let mut set = RoiSet::default();
    set.mark(Roi::new(10, 20));
    set.mark(Roi::new(15, 30));
    assert_eq!(set.len(), 1);
    assert_eq!(set.get(0), Some(&Roi::new(10, 30)));
}

#[test]
fn adjacent_regions_stay_separate() {
    // "Overlapping or close peaks might have contiguous ROIs" - contiguous ROIs
    // remain distinct entries so each peak is reported separately.
    let mut set = RoiSet::default();
    set.mark(Roi::new(10, 20));
    set.mark(Roi::new(21, 30));
    assert_eq!(set.len(), 2);
}

#[test]
fn regions_are_kept_sorted() {
    let mut set = RoiSet::default();
    set.mark(Roi::new(100, 110));
    set.mark(Roi::new(10, 20));
    set.mark(Roi::new(50, 60));
    let starts: Vec<usize> = set.iter().map(|r| r.start).collect();
    assert_eq!(starts, vec![10, 50, 100]);
}

#[test]
fn unmark_can_split_a_region() {
    let mut set = RoiSet::default();
    set.mark(Roi::new(10, 30));
    set.unmark(Roi::new(15, 20));
    assert_eq!(set.len(), 2);
    assert_eq!(set.get(0), Some(&Roi::new(10, 14)));
    assert_eq!(set.get(1), Some(&Roi::new(21, 30)));
}

#[test]
fn unmark_trims_edges_and_removes_swallowed_regions() {
    let mut set = RoiSet::default();
    set.mark(Roi::new(10, 30));
    set.unmark(Roi::new(5, 12));
    assert_eq!(set.get(0), Some(&Roi::new(13, 30)));
    set.unmark(Roi::new(0, 1000));
    assert!(set.is_empty());
}

#[test]
fn clear_removes_only_the_region_under_the_marker() {
    let mut set = RoiSet::default();
    set.mark(Roi::new(10, 20));
    set.mark(Roi::new(40, 50));
    assert!(set.clear_at(45));
    assert!(!set.clear_at(45), "already cleared");
    assert_eq!(set.len(), 1);
    assert_eq!(set.get(0), Some(&Roi::new(10, 20)));
    set.clear_all();
    assert!(set.is_empty());
}

#[test]
fn clearing_a_span_takes_every_region_it_touches() {
    // What a dragged-out selection clears: everything inside it, and a region
    // straddling the edge goes too - it was pointed at, and leaving half of
    // one behind is a stranger result than removing it.
    let mut set = RoiSet::default();
    set.mark(Roi::new(10, 20));
    set.mark(Roi::new(40, 50));
    set.mark(Roi::new(60, 70));
    set.mark(Roi::new(200, 210));

    assert_eq!(set.clear_within(Roi::new(45, 100)), 2, "40-50 and 60-70");
    assert_eq!(set.len(), 2);
    assert_eq!(set.get(0), Some(&Roi::new(10, 20)));
    assert_eq!(set.get(1), Some(&Roi::new(200, 210)));

    // A span touching nothing removes nothing, and says so by returning zero.
    assert_eq!(set.clear_within(Roi::new(100, 150)), 0);
    assert_eq!(set.len(), 2);

    // A single shared channel is enough to count as touched.
    assert_eq!(set.clear_within(Roi::new(20, 20)), 1);
    assert_eq!(set.len(), 1);
}

#[test]
fn lookup_by_channel() {
    let mut set = RoiSet::default();
    set.mark(Roi::new(10, 20));
    set.mark(Roi::new(40, 50));
    assert_eq!(set.at(15), Some(&Roi::new(10, 20)));
    assert_eq!(set.index_at(45), Some(1));
    assert_eq!(set.at(30), None);
}

#[test]
fn indexing_buttons_walk_between_regions() {
    let mut set = RoiSet::default();
    set.mark(Roi::new(10, 20));
    set.mark(Roi::new(40, 50));
    set.mark(Roi::new(80, 90));

    assert_eq!(set.next_after(0).map(|r| r.start), Some(10));
    assert_eq!(set.next_after(15).map(|r| r.start), Some(40));
    assert_eq!(set.next_after(85).map(|r| r.start), None, "no more entries");
    assert_eq!(set.previous_before(85).map(|r| r.start), Some(40));
    assert_eq!(set.previous_before(5), None);
}

#[test]
fn total_marked_channels() {
    let mut set = RoiSet::default();
    set.mark(Roi::new(10, 19));
    set.mark(Roi::new(30, 39));
    assert_eq!(set.marked_channels(), 20);
}

#[test]
fn from_pairs_round_trips_to_pairs() {
    let set = RoiSet::from_pairs([(10usize, 20usize), (40, 50)]);
    assert_eq!(set.len(), 2);
    assert_eq!(set.to_pairs(), vec![(10, 20), (40, 50)]);
}
