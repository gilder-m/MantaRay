//! The deep save-path reviews of 59e3230 (runs/release-0.2.5/deep-forward.md
//! F1-F3 and N1), run against the rig: a count is finished only once a
//! SHOW_DATA taken after the detector stopped has landed, with its clocks,
//! and a network line that timed out once is not left out of step.

use std::time::Duration;

use mantaray_device::rig::{Fault, Rig};
use mantaray_device::{
    CountState, CountWatch, Mcb, Presets, RemoteMcb, TcpTransport, advance,
};

const RATE: f64 = 1000.0;

fn counting(rig: &Rig, live: f64) -> RemoteMcb {
    let mut mcb = RemoteMcb::connect(rig.transport(), 1, "RIG").expect("connect");
    mcb.set_confirm_step(Duration::ZERO);
    rig.set_rate(RATE);
    rig.tick_per_status(0.5);
    mcb.apply_presets(Presets::live(live)).expect("preset");
    mcb.clear().expect("clear");
    mcb.start().expect("start");
    mcb
}

/// A CLI-style WAIT; `arm` runs before each poll. Poll errors are the
/// GUI's: shown, not fatal (`advance_by`).
fn wait(rig: &Rig, mcb: &mut RemoteMcb, mut arm: impl FnMut(&Rig)) -> CountState {
    let mut watch = CountWatch::started(&*mcb);
    for _ in 0..200 {
        match watch.check(&*mcb) {
            CountState::Counting => {}
            other => return other,
        }
        arm(rig);
        std::thread::sleep(Duration::from_millis(260));
        let _ = advance(mcb, 0.5);
    }
    panic!("WAIT did not end");
}

/// The mirror's spectrum is the count the rig finished: its live time and
/// every count.
fn assert_finished(rig: &Rig, mcb: &RemoteMcb) {
    let spectrum = mcb.spectrum();
    let (real, live) = rig.clocks();
    assert!(
        (spectrum.live_time - live).abs() < 1e-9 && (spectrum.real_time - real).abs() < 1e-9,
        "the spectrum says LT {} RT {}; the detector stopped at LT {live} RT {real}",
        spectrum.live_time,
        spectrum.real_time
    );
    // One count of slack: the rig keeps whole counts.
    let expected = RATE * live;
    assert!(
        (spectrum.total_counts() as f64 - expected).abs() < 1.5,
        "the spectrum holds {} counts; the detector counted {expected}",
        spectrum.total_counts()
    );
}

fn arm_at(live: f64, fault: Fault) -> impl FnMut(&Rig) {
    let mut armed = false;
    move |rig: &Rig| {
        if !armed && rig.clocks().1 >= live - 1e-9 {
            armed = true;
            rig.fault(fault.clone());
        }
    }
}

fn data_err(count: usize) -> Fault {
    Fault::Reply {
        verb: "SHOW_DATA".into(),
        reply: "ERR the adapter sent 0 of 64 bytes from 0x0000".into(),
        count,
        execute: true,
    }
}

// ------------------------------------------------------------------ F1

/// F1, the reviewer's repro: the final SHOW_DATA answers ERR. The idle
/// status beside it was taken as the count's end, and the WAIT ended on the
/// spectrum from the fetch before (LT 1.5 s, 1499 counts of 2.0 s / 2000).
#[test]
fn f1_a_failed_data_read_on_the_last_fetch_does_not_end_the_wait_on_the_previous_spectrum() {
    let rig = Rig::new(16);
    let mut mcb = counting(&rig, 2.0);
    let state = wait(&rig, &mut mcb, arm_at(1.5, data_err(1)));
    assert_eq!(state, CountState::Over);
    assert!(!rig.counting());
    assert_finished(&rig, &mcb);
}

/// F1, bounded: a spectrum that never reads again ends the WAIT with an
/// error that says so, after UNREADABLE_READINGS fetches, and the mirror
/// never claims the count it could not read.
#[test]
fn f1_a_spectrum_that_stays_unreadable_ends_the_wait_with_an_error() {
    let rig = Rig::new(16);
    let mut mcb = counting(&rig, 2.0);
    let state = wait(&rig, &mut mcb, arm_at(1.5, data_err(usize::MAX)));
    match state {
        CountState::Unreadable(reason) => assert!(
            reason.contains("spectrum") && reason.contains("0 of 64 bytes"),
            "{reason}"
        ),
        other => panic!("the WAIT ended {other:?} on a spectrum it never read"),
    }
    assert!(mcb.spectrum().live_time < 2.0 - 1e-9);
}

/// A failed spectrum read mid-count keeps the status and spectrum the
/// last good pair, and the next good fetch lands as before.
#[test]
fn f1_a_failed_data_read_mid_count_keeps_the_last_good_pair() {
    let rig = Rig::new(16);
    let mut mcb = counting(&rig, 60.0);
    advance(&mut mcb, 0.5).expect("poll");
    let before = (mcb.status().live_time, mcb.spectrum().live_time);
    rig.fault(data_err(1));
    assert!(advance(&mut mcb, 0.5).is_err(), "the failed read is said");
    assert!(mcb.reading_untrusted().is_some());
    assert_eq!((mcb.status().live_time, mcb.spectrum().live_time), before);
    advance(&mut mcb, 0.5).expect("poll");
    assert!(mcb.reading_untrusted().is_none());
    let (_, live) = rig.clocks();
    assert!((mcb.spectrum().live_time - live).abs() < 1e-9);
    assert!((mcb.status().live_time - live).abs() < 1e-9);
}

// ------------------------------------------------------------------ F2

/// F2, the reviewer's repro: STOP confirmed on the status alone, so the
/// mirror kept the fetch from before it (LT 2.0 s of a detector stopped at
/// 2.4 s).
#[test]
fn f2_a_save_right_after_stop_holds_the_count_the_detector_stopped_at() {
    let rig = Rig::new(16);
    let mut mcb = counting(&rig, 60.0);
    for _ in 0..4 {
        advance(&mut mcb, 0.5).expect("poll");
    }
    rig.tick(0.4);
    rig.take_sent();
    mcb.stop().expect("stop");
    assert!(!rig.counting());
    assert_finished(&rig, &mcb);
    // The one command added to a STOP: a spectrum read after the confirm.
    assert_eq!(rig.take_sent(), ["STOP", "SHOW_STATUS", "SHOW_DATA"]);
}

/// A failed final read is tried again, a bounded number of times.
#[test]
fn f2_one_failed_final_read_is_tried_again() {
    let rig = Rig::new(16);
    let mut mcb = counting(&rig, 60.0);
    advance(&mut mcb, 0.5).expect("poll");
    rig.tick(0.4);
    rig.fault(data_err(1));
    rig.take_sent();
    mcb.stop().expect("stop");
    assert_finished(&rig, &mcb);
    assert!(mcb.reading_untrusted().is_none());
    assert_eq!(
        rig.take_sent(),
        ["STOP", "SHOW_STATUS", "SHOW_DATA", "SHOW_DATA"]
    );
}

/// A final read that keeps failing fails the STOP with a message saying
/// the detector stopped but its spectrum was not read, and the mirror is
/// marked as not up to date.
#[test]
fn f2_a_final_read_that_keeps_failing_fails_the_stop_and_says_why() {
    let rig = Rig::new(16);
    let mut mcb = counting(&rig, 60.0);
    advance(&mut mcb, 0.5).expect("poll");
    rig.tick(0.4);
    rig.fault(data_err(usize::MAX));
    let error = mcb.stop().expect_err("a STOP whose count was never read");
    let message = error.to_string();
    assert!(
        message.contains("stopped") && message.contains("spectrum") && message.contains("0 of 64"),
        "{message}"
    );
    assert!(!rig.counting());
    assert!(mcb.reading_untrusted().is_some());
}

/// The same through a courier (the application's road).
#[test]
fn f2_over_a_courier_too() {
    let rig = Rig::new(16);
    let mut mcb = counting(&rig, 60.0).with_courier();
    rig.tick(1.0);
    mcb.stop().expect("stop");
    assert_finished(&rig, &mcb);
}

// ------------------------------------------------------------------ F3

/// F3, the reviewer's repro: one reply later than the timeout left the TCP
/// line one reply behind for good (12 of 12 later polls refused or not
/// believed). The line now starts afresh after a timeout.
#[test]
fn f3_one_late_tcp_reply_does_not_leave_the_line_out_of_step() {
    let rig = Rig::new(16);
    rig.set_rate(RATE);
    let address = rig.serve();
    let transport = TcpTransport::connect(&address, Duration::from_millis(300)).expect("tcp");
    let mut mcb = RemoteMcb::connect(Box::new(transport), 1, "RIG").expect("connect");
    mcb.set_confirm_step(Duration::ZERO);
    mcb.clear().expect("clear");
    mcb.start().expect("start");
    advance(&mut mcb, 0.5).expect("poll");
    rig.set_latency(Duration::from_millis(600));
    assert!(advance(&mut mcb, 0.5).is_err(), "the late reply times out");
    rig.set_latency(Duration::ZERO);
    let mut bad = Vec::new();
    for _ in 0..12 {
        rig.tick(0.5);
        match advance(&mut mcb, 0.5) {
            Ok(_) if mcb.reading_untrusted().is_none() => {}
            Ok(_) => bad.push(mcb.reading_untrusted().unwrap_or_default().to_string()),
            Err(error) => bad.push(error.to_string()),
        }
    }
    assert!(bad.is_empty(), "{} of 12 polls failed: {bad:?}", bad.len());
    let (_, live) = rig.clocks();
    assert!((mcb.spectrum().live_time - live).abs() < 1e-9);
    // Commands are answered by their own replies again.
    mcb.stop().expect("stop");
    assert!(!rig.counting());
}

// ------------------------------------------------------------------ N1

/// N1: two held fetches in a row (lengths flapping on the UMCBI road) on
/// the fetches that see the count end. The first was set aside; the second
/// had its idle status worked in beside the old channels and clocks, and
/// the WAIT ended on that earlier pair.
#[test]
fn n1_two_held_fetches_in_a_row_do_not_end_the_wait_on_the_earlier_pair() {
    let rig = Rig::new(16);
    let mut mcb = counting(&rig, 2.0);
    let mut armed = false;
    let state = wait(&rig, &mut mcb, |rig| {
        if !armed && rig.clocks().1 >= 1.5 - 1e-9 {
            armed = true;
            for reply in ["DATA 8 1 1 1 1 1 1 1 1", "DATA 4 1 1 1 1"] {
                rig.fault(Fault::Reply {
                    verb: "SHOW_DATA".into(),
                    reply: reply.into(),
                    count: 1,
                    execute: true,
                });
            }
        }
    });
    assert_eq!(state, CountState::Over);
    assert_eq!(mcb.spectrum().len(), 16);
    assert_finished(&rig, &mcb);
}

/// N1, bounded: lengths that never agree end a WAIT as unreadable, never
/// as over on channels from before.
#[test]
fn n1_lengths_that_never_settle_end_the_wait_as_unreadable() {
    let rig = Rig::new(16);
    let mut mcb = counting(&rig, 2.0);
    let mut armed = false;
    let state = wait(&rig, &mut mcb, |rig| {
        if !armed && rig.clocks().1 >= 1.5 - 1e-9 {
            armed = true;
            for n in 0..60 {
                let reply = if n % 2 == 0 {
                    "DATA 8 1 1 1 1 1 1 1 1"
                } else {
                    "DATA 4 1 1 1 1"
                };
                rig.fault(Fault::Reply {
                    verb: "SHOW_DATA".into(),
                    reply: reply.into(),
                    count: 1,
                    execute: true,
                });
            }
        }
    });
    assert!(
        matches!(state, CountState::Unreadable(ref reason) if reason.contains("channel")),
        "{state:?}"
    );
}
