//! The release-gate review of 96b5eee (runs/release-0.2.5/review-device.md),
//! S1 and S2, run against the rig: a WAIT ends only on a reading it can
//! believe, and the spectrum it leaves has counts and clocks from one fetch.

use std::time::Duration;

use mantaray_device::rig::{Fault, Rig};
use mantaray_device::{CountState, CountWatch, Mcb, Presets, RemoteMcb, advance};

fn setup(rate: f64) -> (Rig, RemoteMcb) {
    let rig = Rig::new(16);
    rig.set_rate(rate);
    rig.tick_per_status(0.5);
    let mut mcb = RemoteMcb::connect(rig.transport(), 1, "RIG").expect("connect");
    mcb.set_confirm_step(Duration::ZERO);
    (rig, mcb)
}

/// Runs a CLI-style WAIT; `arm` is called before each poll with the rig.
fn wait(rig: &Rig, mcb: &mut RemoteMcb, mut arm: impl FnMut(&Rig, &RemoteMcb)) -> CountState {
    let mut watch = CountWatch::started(&*mcb);
    for _ in 0..200 {
        match watch.check(&*mcb) {
            CountState::Counting => {}
            other => return other,
        }
        arm(rig, mcb);
        std::thread::sleep(Duration::from_millis(260));
        let _ = advance(mcb, 0.5);
    }
    panic!("WAIT did not end");
}

fn counting(rig: &Rig, rate: f64, live: f64) -> RemoteMcb {
    let mut mcb = RemoteMcb::connect(rig.transport(), 1, "RIG").expect("connect");
    mcb.set_confirm_step(Duration::ZERO);
    rig.set_rate(rate);
    rig.tick_per_status(0.5);
    mcb.apply_presets(Presets::live(live)).expect("preset");
    mcb.clear().expect("clear");
    mcb.start().expect("start");
    mcb
}

fn assert_whole(mcb: &RemoteMcb, rate: f64) {
    let spectrum = mcb.spectrum();
    // One count of slack: the rig keeps whole counts, so a rate times a live
    // time can land a count short. A held fetch is half a second short.
    let expected = rate * spectrum.live_time;
    assert!(
        (spectrum.total_counts() as f64 - expected).abs() <= 1.0,
        "mirror holds {} counts with live time {} s (rate {rate} cps)",
        spectrum.total_counts(),
        spectrum.live_time
    );
}

/// S2, the reviewer's repro. A one-fetch length flap (the UMCBI road
/// truncating a read) on the fetch that sees the count end: the mirror kept
/// the previous fetch's channels but took this fetch's clocks, and the WAIT
/// ended on it. A SAVE wrote 0.5 s fewer counts than its live time said.
#[test]
fn flap_on_the_last_fetch_pairs_stale_counts_with_the_final_live_time() {
    let (rig, mut mcb) = setup(1000.0);
    mcb.apply_presets(Presets::live(2.0)).expect("preset");
    mcb.clear().expect("clear");
    mcb.start().expect("start");
    let mut armed = false;
    let state = wait(&rig, &mut mcb, |rig, _| {
        if !armed && rig.clocks().1 >= 1.5 - 1e-9 {
            armed = true;
            rig.fault(Fault::Reply {
                verb: "SHOW_DATA".into(),
                reply: "DATA 8 1 1 1 1 1 1 1 1".into(),
                count: 1,
                execute: true,
            });
        }
    });
    assert_eq!(state, CountState::Over);
    assert_whole(&mcb, 1000.0);
    // And the count it saved is the whole one, not the reading before it.
    assert!((mcb.spectrum().live_time - 2.0).abs() < 1e-9);
}

/// S1 as the device sees it once the bridge is fixed: a status whose flag
/// could not be read arrives as ERR (`serve.rs`), and the WAIT goes on over
/// it rather than ending with the count still running.
#[test]
fn a_status_the_bridge_cannot_read_the_flag_for_does_not_end_the_wait() {
    let rig = Rig::new(16);
    let mut mcb = counting(&rig, 100.0, 5.0);
    let mut armed = false;
    let state = wait(&rig, &mut mcb, |rig, _| {
        if !armed && rig.clocks().1 >= 2.0 - 1e-9 {
            armed = true;
            rig.fault(Fault::Reply {
                verb: "SHOW_STATUS".into(),
                reply: "ERR the counting flag cannot be read: SHOW_ACTIVE timed out".into(),
                count: 1,
                execute: true,
            });
        }
    });
    assert!(armed);
    assert_eq!(state, CountState::Over);
    assert!(
        !rig.counting(),
        "the WAIT ended while the instrument counts"
    );
    assert!((mcb.spectrum().live_time - 5.0).abs() < 1e-9);
    assert_whole(&mcb, 100.0);
}

/// A single good status after a failed one is believed at once, exactly as
/// before: no "two good in a row" rule, no refusal on a healthy line.
#[test]
fn one_good_status_after_a_failed_one_is_believed_at_once() {
    let rig = Rig::new(16);
    let mut mcb = counting(&rig, 100.0, 60.0);
    advance(&mut mcb, 0.5).expect("poll");
    rig.fault(Fault::Reply {
        verb: "SHOW_STATUS".into(),
        reply: "ERR the counting flag cannot be read: SHOW_ACTIVE timed out".into(),
        count: 1,
        execute: true,
    });
    advance(&mut mcb, 0.5).expect("poll");
    assert!(mcb.reading_untrusted().is_some());
    let before = mcb.readings();
    advance(&mut mcb, 0.5).expect("poll");
    assert_eq!(mcb.readings(), before + 1);
    assert!(mcb.reading_untrusted().is_none());
    assert!(mcb.is_active());
    let (real, live) = rig.clocks();
    assert!((mcb.status().live_time - live).abs() < 1e-9);
    assert!((mcb.status().real_time - real).abs() < 1e-9);
    assert_whole(&mcb, 100.0);
    // The gate asks afresh and gets an answer: a stop goes through.
    mcb.stop().expect("a stop on a healthy line");
    assert!(!rig.counting());
}

/// A single flap mid-count followed by one whole fetch: the whole fetch lands
/// at once, as before, and the spectrum's clocks are that fetch's.
#[test]
fn one_whole_fetch_after_a_flap_lands_at_once() {
    let rig = Rig::new(16);
    let mut mcb = counting(&rig, 1000.0, 60.0);
    advance(&mut mcb, 0.5).expect("poll");
    rig.fault(Fault::Reply {
        verb: "SHOW_DATA".into(),
        reply: "DATA 8 1 1 1 1 1 1 1 1".into(),
        count: 1,
        execute: true,
    });
    advance(&mut mcb, 0.5).expect("poll");
    assert_eq!(mcb.spectrum().len(), 16);
    assert_whole(&mcb, 1000.0);
    advance(&mut mcb, 0.5).expect("poll");
    assert_eq!(mcb.spectrum().len(), 16);
    let (_, live) = rig.clocks();
    assert!((mcb.spectrum().live_time - live).abs() < 1e-9);
    assert_whole(&mcb, 1000.0);
    assert!(mcb.reading_untrusted().is_none());
}

/// Lengths that never agree twice are set aside once, not forever: the
/// readings go on (a WAIT can still end), and the spectrum keeps the clocks of
/// the channels it holds.
#[test]
fn a_run_of_flaps_is_set_aside_once_and_keeps_clocks_with_channels() {
    let rig = Rig::new(16);
    let mut mcb = counting(&rig, 1000.0, 60.0);
    advance(&mut mcb, 0.5).expect("poll");
    let landed = mcb.spectrum().live_time;
    let before = mcb.readings();
    for reply in [
        "DATA 8 1 1 1 1 1 1 1 1",
        "DATA 4 1 1 1 1",
        "DATA 8 1 1 1 1 1 1 1 1",
    ] {
        rig.fault(Fault::Reply {
            verb: "SHOW_DATA".into(),
            reply: reply.into(),
            count: 1,
            execute: true,
        });
        advance(&mut mcb, 0.5).expect("poll");
    }
    assert_eq!(
        mcb.readings(),
        before + 2,
        "only the first flap is set aside"
    );
    assert_eq!(mcb.spectrum().len(), 16);
    assert!((mcb.spectrum().live_time - landed).abs() < 1e-9);
    assert_whole(&mcb, 1000.0);
    // The status says where the instrument is; the spectrum, where its
    // counts were read.
    assert!(mcb.status().live_time > landed);
}
