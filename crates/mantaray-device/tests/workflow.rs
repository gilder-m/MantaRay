//! Detector lists, sample-changer batches, alarms and the dashboard tiles.

use mantaray_device::{
    AlarmKind, AlarmLimits, AlarmMonitor, DetectorEntry, DetectorKind, DetectorList, JobState, Mcb,
    Presets, SampleChanger, SampleJob, SampleQueue, SimulatedMcb, advance, tile,
};

#[test]
fn a_detector_list_is_a_pick_list_of_numbered_entries() {
    let mut list = DetectorList::new("M32MCA");
    list.push(DetectorEntry::simulated(1, "SIM-01"));
    list.push(DetectorEntry {
        number: 12,
        name: "IDM-200".into(),
        model: "IDM-200".into(),
        kind: DetectorKind::Simulated,
        channels: 16384,
        description: "portable HPGe".into(),
        serial: String::new(),
    });

    assert_eq!(list.len(), 2);
    assert_eq!(list.get(12).map(|e| e.name.as_str()), Some("IDM-200"));
    assert!(list.get(3).is_none());
    assert_eq!(list.numbers(), vec![1, 12]);

    // Numbers are unique: pushing the same number replaces the entry.
    list.push(DetectorEntry::simulated(1, "SIM-01b"));
    assert_eq!(list.len(), 2);
    assert_eq!(list.get(1).map(|e| e.name.as_str()), Some("SIM-01b"));

    assert!(list.remove(1));
    assert!(!list.remove(1));
    assert_eq!(list.len(), 1);
}

#[test]
fn a_detector_list_round_trips_as_json() {
    let mut list = DetectorList::new("pick");
    list.push(DetectorEntry::simulated(1, "A"));
    list.push(DetectorEntry::simulated(2, "B"));
    let text = list.to_json().unwrap();
    let back = DetectorList::from_json(&text).unwrap();
    assert_eq!(back, list);
}

#[test]
fn detector_numbers_are_limited_to_the_maestro_range() {
    let mut list = DetectorList::new("pick");
    assert!(
        list.try_push(DetectorEntry::simulated(0, "buffer"))
            .is_err()
    );
    assert!(
        list.try_push(DetectorEntry::simulated(1000, "too big"))
            .is_err()
    );
    assert!(list.try_push(DetectorEntry::simulated(999, "last")).is_ok());
}

#[test]
fn a_sample_queue_walks_through_its_jobs() {
    let mut queue = SampleQueue::default();
    for index in 1..=3 {
        queue.push(SampleJob {
            id: format!("S{index:03}"),
            description: format!("sample {index}"),
            presets: Presets {
                live_time: Some(10.0),
                ..Presets::default()
            },
            quantity: 1.0,
            unit: "kg".into(),
            state: JobState::Pending,
        });
    }
    assert_eq!(queue.len(), 3);
    assert_eq!(queue.remaining(), 3);
    assert_eq!(queue.current().map(|job| job.id.as_str()), Some("S001"));
    assert!((queue.progress() - 0.0).abs() < 1e-9);

    queue.begin_current();
    assert_eq!(
        queue.current().map(|job| job.state.clone()),
        Some(JobState::Counting)
    );
    queue.complete_current();
    assert_eq!(queue.current().map(|job| job.id.as_str()), Some("S002"));
    assert!((queue.progress() - 1.0 / 3.0).abs() < 1e-9);

    queue.fail_current("detector fault");
    assert_eq!(queue.current().map(|job| job.id.as_str()), Some("S003"));
    queue.complete_current();
    assert!(queue.is_complete());
    assert!(queue.current().is_none());
    assert_eq!(queue.completed(), 2);
    assert_eq!(queue.failed(), 1);
}

#[test]
fn the_sample_changer_handshake_takes_time() {
    let mut changer = SampleChanger::new(10);
    assert!(changer.is_ready());
    assert_eq!(changer.position(), 0);

    changer.request_change();
    assert!(
        !changer.is_ready(),
        "the sample-ready signal drops while moving"
    );
    changer.poll(0.5);
    assert!(!changer.is_ready());
    changer.poll(3.0);
    assert!(changer.is_ready(), "and returns when the changer arrives");
    assert_eq!(changer.position(), 1);

    // Past the last position the changer reports that it is out of samples.
    for _ in 0..12 {
        changer.request_change();
        changer.poll(5.0);
    }
    assert!(changer.is_exhausted());
}

#[test]
fn a_batch_runs_a_queue_against_a_detector() {
    let mut mcb = SimulatedMcb::builder(1, "SIM").build();
    let mut queue = SampleQueue::default();
    for index in 0..2 {
        queue.push(SampleJob {
            id: format!("B{index}"),
            description: "batch".into(),
            presets: Presets {
                real_time: Some(5.0),
                ..Presets::default()
            },
            quantity: 1.0,
            unit: String::new(),
            state: JobState::Pending,
        });
    }
    let mut changer = SampleChanger::new(4);
    let mut finished = 0;

    while !queue.is_complete() {
        let job = queue.current().cloned().unwrap();
        mcb.stop().unwrap();
        mcb.set_presets(job.presets).unwrap();
        mcb.clear().unwrap();
        mcb.start().unwrap();
        queue.begin_current();
        let mut guard = 0;
        while mcb.is_active() && guard < 100 {
            advance(&mut mcb, 1.0).unwrap();
            guard += 1;
        }
        assert!(!mcb.is_active(), "the preset stopped the count");
        assert!(mcb.status().real_time >= 5.0);
        queue.complete_current();
        finished += 1;
        changer.request_change();
        changer.poll(5.0);
    }
    assert_eq!(finished, 2);
    assert_eq!(changer.position(), 2);
}

#[test]
fn alarms_trip_on_dead_time_and_rate_and_latch_until_reset() {
    let mut monitor = AlarmMonitor::new(AlarmLimits {
        max_dead_time_percent: Some(30.0),
        max_count_rate: Some(10_000.0),
        min_count_rate: None,
        max_activity: None,
    });
    let mut mcb = SimulatedMcb::builder(1, "hot")
        .source(mantaray_device::SimulatedSource {
            continuum_cps: 200_000.0,
            ..Default::default()
        })
        .build();
    mcb.start().unwrap();
    mcb.poll(20.0).unwrap();

    let tripped = monitor.evaluate(&mcb.status()).to_vec();
    assert!(tripped.contains(&AlarmKind::DeadTime), "{tripped:?}");
    assert!(tripped.contains(&AlarmKind::HighCountRate), "{tripped:?}");
    assert!(monitor.is_tripped());

    // A quiet detector does not clear a latched alarm.
    let mut quiet = SimulatedMcb::builder(2, "quiet").build();
    quiet.start().unwrap();
    quiet.poll(5.0).unwrap();
    monitor.evaluate(&quiet.status());
    assert!(monitor.is_tripped(), "alarms latch");
    monitor.reset();
    assert!(!monitor.is_tripped());
    assert!(monitor.evaluate(&quiet.status()).is_empty());
}

#[test]
fn a_low_rate_alarm_catches_a_dead_detector() {
    let monitor_limits = AlarmLimits {
        min_count_rate: Some(1.0),
        ..AlarmLimits::default()
    };
    let mut monitor = AlarmMonitor::new(monitor_limits);
    let mut mcb = SimulatedMcb::builder(1, "empty")
        .source(mantaray_device::SimulatedSource {
            lines: Vec::new(),
            continuum_cps: 0.0,
            ..Default::default()
        })
        .build();
    mcb.start().unwrap();
    mcb.poll(10.0).unwrap();
    assert!(
        monitor
            .evaluate(&mcb.status())
            .contains(&AlarmKind::LowCountRate)
    );
}

#[test]
fn dashboard_tiles_summarise_a_detector() {
    let mut mcb = SimulatedMcb::builder(4, "SIM-04").build();
    mcb.presets_mut().live_time = Some(100.0);
    mcb.start().unwrap();
    advance(&mut mcb, 25.0).unwrap();

    let tile = tile(&mcb);
    assert_eq!(tile.number, 4);
    assert_eq!(tile.name, "SIM-04");
    assert!(tile.active);
    assert!(tile.live_time > 24.0);
    assert!(tile.count_rate > 0.0);
    let progress = tile
        .preset_progress
        .expect("a live-time preset gives progress");
    assert!(progress > 0.2 && progress < 0.3, "progress {progress}");
    assert!(
        tile.status_text.to_lowercase().contains("acquir"),
        "{}",
        tile.status_text
    );
}
