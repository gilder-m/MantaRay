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
    // Re-pinned for deep review N1: the later flaps are readings that are
    // not believed, status and all, so the status stays with the spectrum
    // (an idle status beside old channels ended a WAIT on them) and the
    // reading is marked until a fetch lands.
    assert!((mcb.status().live_time - landed).abs() < 1e-9);
    assert!(
        mcb.reading_untrusted()
            .is_some_and(|reason| reason.contains("channel count"))
    );
}

/// The ERR the bridge gives for a flag it cannot read (S1).
const FLAG_UNREADABLE: &str = "ERR the counting flag cannot be read: SHOW_ACTIVE timed out";

fn unreadable_flag(rig: &Rig, count: usize) {
    rig.fault(Fault::Reply {
        verb: "SHOW_STATUS".into(),
        reply: FLAG_UNREADABLE.into(),
        count,
        execute: true,
    });
}

/// Review of 4597899 (Lucius, S1 x rule 9): one unreadable flag in a STOP's
/// confirm uses up one of its reads; it no longer fails a STOP on a detector
/// that has stopped, so a job's START / WAIT / STOP / SAVE still saves.
#[test]
fn one_unreadable_flag_in_a_stop_confirm_uses_up_one_read() {
    let rig = Rig::new(16);
    let mut mcb = counting(&rig, 100.0, 1.0);
    for _ in 0..10 {
        advance(&mut mcb, 0.5).expect("poll");
    }
    assert!(!rig.counting(), "the preset ended the count");
    assert!(!mcb.is_active());
    unreadable_flag(&rig, 1);
    let stopped = mcb.stop();
    assert!(!rig.counting());
    assert!(
        stopped.is_ok(),
        "STOP on a stopped detector failed on one unreadable flag: {stopped:?}"
    );
}

/// The same for the host-side preset stop, which shares the confirm: one
/// unreadable flag while a reached uncertainty preset stops a counting
/// detector does not fail the stop (what `advance` does, one step at a time
/// so the fault lands in the confirm and not in the poll).
#[test]
fn one_unreadable_flag_in_a_preset_stop_confirm_uses_up_one_read() {
    let rig = Rig::new(16);
    let mut mcb = RemoteMcb::connect(rig.transport(), 1, "RIG").expect("connect");
    mcb.set_confirm_step(Duration::ZERO);
    rig.set_rate(1000.0);
    rig.tick_per_status(0.5);
    let mut presets = Presets::default();
    presets.set_uncertainty(50.0, 0, 15);
    mcb.apply_presets(presets).expect("preset");
    mcb.clear().expect("clear");
    mcb.start().expect("start");
    for _ in 0..20 {
        mcb.poll(0.5).expect("poll");
        if mcb.preset_reached().is_some() {
            break;
        }
    }
    assert!(mcb.preset_reached().is_some(), "the uncertainty preset is reached");
    assert!(rig.counting(), "a host-side preset is the host's to stop");
    unreadable_flag(&rig, 1);
    let stopped = mcb.stop_at_preset();
    assert!(
        stopped.is_ok(),
        "the preset stop failed on one unreadable flag: {stopped:?}"
    );
    assert!(!rig.counting());
    assert!(!mcb.is_active());
}

/// Still bounded: ten unreadable flags fail the STOP, saying why.
#[test]
fn a_stop_confirm_that_never_reads_the_flag_still_fails() {
    let rig = Rig::new(16);
    let mut mcb = counting(&rig, 100.0, 60.0);
    advance(&mut mcb, 0.5).expect("poll");
    unreadable_flag(&rig, 10);
    let stopped = mcb.stop();
    let error = stopped.expect_err("ten unreadable flags confirm nothing").to_string();
    assert!(error.contains("could not confirm"), "{error}");
    assert!(error.contains("SHOW_ACTIVE timed out"), "{error}");
}

/// Still bounded: unreadable flags among reads that say counting fail the
/// STOP as "did not stop".
#[test]
fn a_stop_confirm_that_never_reads_idle_still_fails() {
    let rig = Rig::new(16);
    let mut mcb = counting(&rig, 100.0, 60.0);
    advance(&mut mcb, 0.5).expect("poll");
    rig.fault(Fault::StopLag { reads: 1000 });
    unreadable_flag(&rig, 3);
    assert!(mcb.stop().is_err(), "a flag that never says idle confirms nothing");
}

/// Lucius's S2 NOTE on 1894648: two flaps on the last two fetches. Closed by
/// deep review N1 (230ca38), kept here so it stays closed.
#[test]
fn two_flaps_on_the_last_two_fetches_do_not_end_the_wait_on_an_old_spectrum() {
    let (rig, mut mcb) = setup(1000.0);
    mcb.apply_presets(Presets::live(2.0)).expect("preset");
    mcb.clear().expect("clear");
    mcb.start().expect("start");
    let mut stage = 0_u32;
    let state = wait(&rig, &mut mcb, |rig, _| {
        let live = rig.clocks().1;
        let reply = match stage {
            0 if live >= 1.0 - 1e-9 => "DATA 8 1 1 1 1 1 1 1 1",
            1 if live >= 1.5 - 1e-9 => "DATA 4 1 1 1 1",
            _ => return,
        };
        stage += 1;
        rig.fault(Fault::Reply {
            verb: "SHOW_DATA".into(),
            reply: reply.into(),
            count: 1,
            execute: true,
        });
    });
    assert_eq!(stage, 2, "both flaps were armed");
    assert_eq!(state, CountState::Over);
    assert!(!rig.counting());
    assert!((rig.clocks().1 - 2.0).abs() < 1e-9, "the rig finished its preset");
    // Counts and clocks are from one fetch (the S2 fix holds)...
    assert_whole(&mcb, 1000.0);
    // ...but not from the fetch that ended the count.
    assert!(
        (mcb.spectrum().live_time - 2.0).abs() < 1e-9,
        "the WAIT ended on a spectrum with live time {} s; the instrument finished 2 s",
        mcb.spectrum().live_time
    );
}

/// The rig's transport, with every SHOW_DATA after a STOP failing on the line
/// itself (the bridge's 10 s exchange timeout), counting those tries.
struct LineFailsAfterStop {
    inner: Box<dyn mantaray_device::Transport>,
    stopped: bool,
    data_tries: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

impl mantaray_device::Transport for LineFailsAfterStop {
    fn exchange(&mut self, command: &str) -> Result<String, mantaray_device::DeviceError> {
        if command == "STOP" {
            self.stopped = true;
        }
        if command == "SHOW_DATA" && self.stopped {
            self.data_tries
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            return Err(mantaray_device::DeviceError::Connection {
                address: "rig".into(),
                detail: "no answer to \"SHOW_DATA\" within 10 seconds".into(),
            });
        }
        self.inner.exchange(command)
    }

    fn peer(&self) -> String {
        self.inner.peer()
    }
}

/// Review of audit/release-fix-save S-1 (Lucius): a line error on the final
/// fetch after a confirmed STOP is not tried again, because each try of a
/// hung line costs the bridge's whole 10 s timeout (a STOP click waited 30 s).
/// The STOP still fails, saying the detector stopped.
#[test]
fn a_line_error_on_the_final_fetch_is_not_retried() {
    let rig = Rig::new(16);
    let data_tries = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let transport = LineFailsAfterStop {
        inner: rig.transport(),
        stopped: false,
        data_tries: std::sync::Arc::clone(&data_tries),
    };
    let mut mcb = RemoteMcb::connect(Box::new(transport), 1, "RIG").expect("connect");
    mcb.set_confirm_step(Duration::ZERO);
    rig.set_rate(1000.0);
    rig.tick_per_status(0.5);
    mcb.apply_presets(Presets::live(60.0)).expect("preset");
    mcb.clear().expect("clear");
    mcb.start().expect("start");
    advance(&mut mcb, 0.5).expect("poll");
    let error = mcb.stop().expect_err("the final spectrum was never read").to_string();
    assert!(error.contains("the detector stopped"), "{error}");
    assert_eq!(
        data_tries.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "a timed-out line is asked for the final spectrum once"
    );
    assert!(mcb.reading_untrusted().is_some());
}

/// The case F2's retry is for keeps it: an ERR reply to the final SHOW_DATA
/// (the 926's "adapter sent 0 of 64 bytes") is tried again, and the next
/// one lands.
#[test]
fn an_err_reply_on_the_final_fetch_is_still_retried() {
    let rig = Rig::new(16);
    let mut mcb = counting(&rig, 1000.0, 60.0);
    advance(&mut mcb, 0.5).expect("poll");
    rig.fault(Fault::Reply {
        verb: "SHOW_DATA".into(),
        reply: "ERR the adapter sent 0 of 64 bytes from 0x0000".into(),
        count: 1,
        execute: true,
    });
    mcb.stop().expect("the second try reads the final spectrum");
    assert!(mcb.reading_untrusted().is_none());
    assert_whole(&mcb, 1000.0);
}

/// DEV-1 (swarm 11/8, mantaray-test-gaps.md): a `DATA 0` answer to the final
/// fetch after STOP is believed, so the mirror keeps its old channels beside
/// the final clocks and a SAVE writes stale counts. An empty final DATA has
/// to count as an unreadable fetch (tried again, then the STOP fails as F2).
#[test]
fn an_empty_final_fetch_is_not_believed() {
    let rig = Rig::new(16);
    let mut mcb = counting(&rig, 1000.0, 60.0);
    advance(&mut mcb, 0.5).expect("poll");
    // The detector counts on after the last poll, so the final spectrum
    // differs from the one the mirror holds.
    rig.tick(0.4);
    rig.fault(Fault::Reply {
        verb: "SHOW_DATA".into(),
        reply: "DATA 0".into(),
        count: 1,
        execute: true,
    });
    mcb.stop().expect("the second try reads the final spectrum");
    assert_eq!(mcb.spectrum().len(), 16);
    assert_whole(&mcb, 1000.0);
}

/// DEV-2 (swarm 11/8): a readable status read made for a preset change (the
/// gate's) clears the mark a failed SHOW_DATA set, though no spectrum has
/// been read since. Only a fetch that lands may clear it.
#[test]
#[ignore = "DEV-2 open"]
fn a_gate_status_read_does_not_clear_a_failed_fetch_mark() {
    let rig = Rig::new(16);
    let mut mcb = RemoteMcb::connect(rig.transport(), 1, "RIG").expect("connect");
    mcb.set_confirm_step(Duration::ZERO);
    rig.fault(Fault::Reply {
        verb: "SHOW_DATA".into(),
        reply: "ERR the adapter sent 0 of 64 bytes from 0x0000".into(),
        count: 1,
        execute: true,
    });
    // The failed fetch is the poll's error.
    let _ = advance(&mut mcb, 0.5);
    assert!(mcb.reading_untrusted().is_some(), "the failed fetch marks it");
    mcb.apply_presets(Presets::live(5.0)).expect("preset");
    assert!(
        mcb.reading_untrusted().is_some(),
        "a status read is no spectrum read: the mark must stay until a fetch lands"
    );
}
