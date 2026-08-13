//! The remote-instrument layer, proven without hardware.
//!
//! Two levels: the wire protocol against a scripted transport (every byte
//! checked), and a full round trip over real TCP against a served simulator -
//! the same code path a bench instrument will use, minus the bench.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use mantaray_device::{AnyMcb, Mcb, MockTransport, RemoteMcb, SimulatedMcb, TcpTransport, advance};

/// The handshake every script starts with: configuration, status, data.
const HANDSHAKE: usize = 3;

/// A scripted instrument holding no presets.
///
/// Connecting also asks what presets the instrument is already holding, so
/// that answer is spliced in after the handshake and before whatever the test
/// does next. "None set" leaves every case behaving as it did before the
/// question existed; a test that cares scripts its own.
fn connect_scripted(pairs: &[(&str, &str)]) -> RemoteMcb {
    let mut script = pairs.to_vec();
    script.insert(
        pairs.len().min(HANDSHAKE),
        ("SHOW_PRESETS", "PRESETS REAL=0.00 LIVE=0.00 PEAK=0 INTEG=0"),
    );
    connect_exactly(&script)
}

/// A scripted instrument, with no presets answer spliced in.
fn connect_exactly(pairs: &[(&str, &str)]) -> RemoteMcb {
    let transport = MockTransport::scripted(pairs);
    RemoteMcb::connect(Box::new(transport), 7, "BENCH-01").expect("connect")
}

#[test]
fn connecting_learns_what_the_instrument_is() {
    let remote = connect_scripted(&[
        (
            "SHOW_CONFIGURATION",
            "MODEL=DSPEC-50 SERIAL=1234 FIRMWARE=v1.2 CHANNELS=4096",
        ),
        (
            "SHOW_STATUS",
            "RT=0.00 LT=0.00 DT=0.0% ICR=0 ACTIVE=0 TOTAL=0",
        ),
        ("SHOW_DATA", "DATA 4 0 0 0 0"),
    ]);
    assert_eq!(remote.identity().model, "DSPEC-50");
    assert_eq!(remote.identity().serial, "1234");
    assert_eq!(remote.identity().channels, 4096);
    assert_eq!(remote.identity().number, 7);
    assert!(!remote.is_active());
}

#[test]
fn a_malformed_data_reply_is_an_error_not_an_abort() {
    // One corrupt line over TCP or the pipe must surface as an error the
    // caller can show. Before this, a huge declared count aborted the whole
    // process inside Vec::with_capacity, and a garbled channel word quietly
    // became 0 counts.
    for bad in [
        "DATA 1000000000000000", // would try to allocate petabytes
        "DATA 2 5 x",            // a word that is not a count
        "DATA 4 1 2",            // fewer words than declared
        "DATA nonsense",         // no count at all
    ] {
        let mut remote = connect_scripted(&[
            (
                "SHOW_CONFIGURATION",
                "MODEL=M SERIAL=S FIRMWARE=F CHANNELS=8",
            ),
            ("SHOW_STATUS", "RT=0 LT=0 DT=0% ICR=0 ACTIVE=0 TOTAL=0"),
            ("SHOW_DATA", "DATA 2 3 4"),
            ("SHOW_STATUS", "RT=1 LT=1 DT=0% ICR=0 ACTIVE=0 TOTAL=0"),
            ("SHOW_DATA", bad),
        ]);
        let error = remote.poll(1.0).expect_err(bad);
        assert!(
            error.to_string().contains("DATA"),
            "{bad:?} should be refused by name, got: {error}"
        );
        assert_eq!(
            remote.spectrum().channels,
            vec![3, 4],
            "the last good spectrum must survive a corrupt reply"
        );
    }
}

#[test]
fn connecting_learns_the_presets_the_instrument_is_already_holding() {
    // The bug this pins, found on the bench 926 on 2026-08-06: a preset set
    // in one session stays in the instrument's registers, but mantaray only
    // ever wrote presets and never read them. A restarted application showed
    // an empty Presets tab while the instrument held a 300 s live preset -
    // and then answered START without counting, because the preset was
    // already satisfied. Silent from the operator's side, twice over.
    let remote = connect_exactly(&[
        (
            "SHOW_CONFIGURATION",
            "MODEL=M SERIAL=S FIRMWARE=F CHANNELS=8",
        ),
        (
            "SHOW_STATUS",
            "RT=310.90 LT=300.00 DT=3.51% ICR=0 ACTIVE=0 TOTAL=0",
        ),
        ("SHOW_DATA", "DATA 2 0 0"),
        // What the bench 926 was actually holding.
        (
            "SHOW_PRESETS",
            "PRESETS REAL=0.00 LIVE=300.00 PEAK=0 INTEG=0",
        ),
    ]);
    assert_eq!(remote.presets().live_time, Some(300.0));
    assert_eq!(
        remote.presets().real_time,
        None,
        "zero means none set, not a preset of zero"
    );
    assert_eq!(remote.presets().roi_peak, None);
    assert_eq!(remote.presets().roi_integral, None);
}

#[test]
fn an_instrument_that_cannot_say_leaves_the_presets_empty() {
    // An older bridge does not know SHOW_PRESETS and refuses it. That must
    // still connect, with no presets shown - exactly as before the question
    // existed - rather than failing to open the detector at all.
    let remote = connect_exactly(&[
        (
            "SHOW_CONFIGURATION",
            "MODEL=M SERIAL=S FIRMWARE=F CHANNELS=8",
        ),
        ("SHOW_STATUS", "RT=0 LT=0 DT=0% ICR=0 ACTIVE=0 TOTAL=0"),
        ("SHOW_DATA", "DATA 2 0 0"),
        (
            "SHOW_PRESETS",
            "ERR SHOW_PRESETS is not a command this bridge knows",
        ),
    ]);
    assert!(
        remote.presets().is_empty(),
        "nothing is known, so nothing is claimed"
    );
}

#[test]
fn starting_against_a_preset_already_reached_is_refused_by_name() {
    // What the 926 does instead: it answers START, leaves the clocks where
    // they are, and never counts. The refusal has to name the preset, or the
    // operator is left with a Start button that appears to work.
    let mut remote = connect_exactly(&[
        (
            "SHOW_CONFIGURATION",
            "MODEL=M SERIAL=S FIRMWARE=F CHANNELS=8",
        ),
        (
            "SHOW_STATUS",
            "RT=310.90 LT=300.00 DT=3.51% ICR=0 ACTIVE=0 TOTAL=0",
        ),
        ("SHOW_DATA", "DATA 2 0 0"),
        (
            "SHOW_PRESETS",
            "PRESETS REAL=0.00 LIVE=300.00 PEAK=0 INTEG=0",
        ),
        // No START is scripted: sending one would be the bug.
    ]);
    let error = remote.start().expect_err("a satisfied preset cannot start");
    let text = error.to_string();
    assert!(text.contains("Live time"), "should name the preset: {text}");
    assert!(
        text.contains("clear") || text.contains("change"),
        "should say what to do about it: {text}"
    );
    assert!(!remote.is_active());
}

#[test]
fn the_wrong_instrument_is_refused_rather_than_opened() {
    // A saved detector remembers a route - a number, or a position on the USB
    // bus - and a route is not an identity. Plug in a second adapter, or
    // replug the one there, and the same route can reach a different
    // instrument; two of a model answer identically apart from the serial, so
    // nothing downstream would notice it had happened.
    let transport = MockTransport::scripted(&[(
        "SHOW_CONFIGURATION",
        "MODEL=0926-001 SERIAL=11217584 FIRMWARE=F CHANNELS=4096",
    )]);
    let outcome =
        RemoteMcb::connect_expecting(Box::new(transport), 1, "Bench HPGe", Some("08134079"));
    let text = match outcome {
        Ok(_) => panic!("a different instrument must not open under this entry"),
        Err(error) => error.to_string(),
    };
    assert!(
        text.contains("11217584"),
        "should say what answered: {text}"
    );
    assert!(text.contains("08134079"), "and what was expected: {text}");
}

#[test]
fn the_expected_instrument_opens_normally() {
    let remote = RemoteMcb::connect_expecting(
        Box::new(MockTransport::scripted(&[
            (
                "SHOW_CONFIGURATION",
                "MODEL=0926-001 SERIAL=08134079 FIRMWARE=F CHANNELS=8",
            ),
            ("SHOW_STATUS", "RT=0 LT=0 DT=0% ICR=0 ACTIVE=0 TOTAL=0"),
            ("SHOW_DATA", "DATA 2 0 0"),
            ("SHOW_PRESETS", "PRESETS REAL=0.00 LIVE=0.00 PEAK=0 INTEG=0"),
        ])),
        1,
        "Bench HPGe",
        // Case and surrounding space are not a different instrument.
        Some("  08134079  "),
    )
    .expect("the right instrument opens");
    assert_eq!(remote.identity().serial, "08134079");
}

#[test]
fn an_instrument_that_reports_no_serial_is_not_treated_as_a_mismatch() {
    // Some routes report no serial at all. That is nothing to check against,
    // not evidence of the wrong instrument - refusing there would lock the
    // operator out of a detector that is working perfectly well.
    let remote = RemoteMcb::connect_expecting(
        Box::new(MockTransport::scripted(&[
            (
                "SHOW_CONFIGURATION",
                "MODEL=0926-001 SERIAL= FIRMWARE=F CHANNELS=8",
            ),
            ("SHOW_STATUS", "RT=0 LT=0 DT=0% ICR=0 ACTIVE=0 TOTAL=0"),
            ("SHOW_DATA", "DATA 2 0 0"),
            ("SHOW_PRESETS", "PRESETS REAL=0.00 LIVE=0.00 PEAK=0 INTEG=0"),
        ])),
        1,
        "Bench HPGe",
        Some("08134079"),
    );
    assert!(
        remote.is_ok(),
        "a missing serial is nothing to check, not a mismatch: {}",
        remote
            .err()
            .map(|error| error.to_string())
            .unwrap_or_default()
    );
}

#[test]
fn renaming_reaches_the_saved_spectrum_too() {
    // The name is what a file saved from this window says it came from, so a
    // rename that stops at the pick list leaves every later save wrong.
    let mut remote = connect_scripted(&[
        (
            "SHOW_CONFIGURATION",
            "MODEL=M SERIAL=S FIRMWARE=F CHANNELS=8",
        ),
        ("SHOW_STATUS", "RT=0 LT=0 DT=0% ICR=0 ACTIVE=0 TOTAL=0"),
        ("SHOW_DATA", "DATA 2 0 0"),
    ]);
    assert_eq!(remote.spectrum().detector_name, "BENCH-01");
    remote.set_name("Bench HPGe");
    assert_eq!(remote.identity().name, "Bench HPGe");
    assert_eq!(remote.spectrum().detector_name, "Bench HPGe");
    assert_eq!(remote.spectrum().detector_description, "Bench HPGe");
}

#[test]
fn starting_sends_the_start_command_and_nothing_else() {
    let mut remote = connect_scripted(&[
        (
            "SHOW_CONFIGURATION",
            "MODEL=M SERIAL=S FIRMWARE=F CHANNELS=8",
        ),
        ("SHOW_STATUS", "RT=0 LT=0 DT=0% ICR=0 ACTIVE=0 TOTAL=0"),
        ("SHOW_DATA", "DATA 2 0 0"),
        ("START", "OK"),
    ]);
    remote.start().expect("start");
    assert!(remote.is_active(), "the mirror should follow the command");
}

#[test]
fn starting_stamps_the_measurement_date_and_clear_takes_it_back() {
    let mut remote = connect_scripted(&[
        (
            "SHOW_CONFIGURATION",
            "MODEL=M SERIAL=S FIRMWARE=F CHANNELS=8",
        ),
        ("SHOW_STATUS", "RT=0 LT=0 DT=0% ICR=0 ACTIVE=0 TOTAL=0"),
        ("SHOW_DATA", "DATA 2 0 0"),
        ("START", "OK"),
        ("STOP", "OK"),
        ("START", "OK"),
        ("CLEAR", "OK"),
    ]);
    assert!(
        remote.spectrum().start_time.is_none(),
        "nothing has been measured yet"
    );
    remote.start().expect("start");
    let stamped = remote
        .spectrum()
        .start_time
        .expect("starting is what sets the measurement date");
    remote.stop().expect("stop");
    remote.start().expect("resume");
    assert_eq!(
        remote.spectrum().start_time,
        Some(stamped),
        "a resumed count keeps the date it started"
    );
    remote.clear().expect("clear");
    assert!(
        remote.spectrum().start_time.is_none(),
        "cleared data has not been measured"
    );
}

#[test]
fn a_spectrum_knows_which_detector_it_came_from_even_after_a_resize() {
    let mut remote = connect_scripted(&[
        (
            "SHOW_CONFIGURATION",
            "MODEL=M SERIAL=S FIRMWARE=F CHANNELS=8",
        ),
        ("SHOW_STATUS", "RT=0 LT=0 DT=0% ICR=0 ACTIVE=0 TOTAL=0"),
        ("SHOW_DATA", "DATA 2 0 0"),
        // The next poll finds the conversion gain changed under us.
        ("SHOW_STATUS", "RT=1 LT=1 DT=0% ICR=0 ACTIVE=0 TOTAL=10"),
        ("SHOW_DATA", "DATA 4 1 2 3 4"),
    ]);
    assert_eq!(remote.spectrum().detector_id, 7);
    assert_eq!(remote.spectrum().detector_name, "BENCH-01");
    assert_eq!(remote.spectrum().detector_description, "BENCH-01");
    remote.poll(1.0).expect("poll");
    assert_eq!(remote.spectrum().len(), 4, "the counts are the new size");
    assert_eq!(
        remote.spectrum().detector_description,
        "BENCH-01",
        "whose spectrum this is survives the resize"
    );
}

#[test]
fn presets_are_pushed_as_the_documented_commands() {
    let mut remote = connect_scripted(&[
        (
            "SHOW_CONFIGURATION",
            "MODEL=M SERIAL=S FIRMWARE=F CHANNELS=8",
        ),
        ("SHOW_STATUS", "RT=0 LT=0 DT=0% ICR=0 ACTIVE=0 TOTAL=0"),
        ("SHOW_DATA", "DATA 2 0 0"),
        ("SET_PRESET_CLEAR", "OK"),
        ("SET_PRESET_REAL 500", "OK"),
        ("SET_PRESET_LIVE 300", "OK"),
    ]);
    let presets = mantaray_device::Presets {
        real_time: Some(500.0),
        live_time: Some(300.0),
        ..Default::default()
    };
    remote.set_presets(presets).expect("presets");
    assert_eq!(remote.presets().live_time, Some(300.0));
}

#[test]
fn a_rejection_comes_back_as_the_error_it_is() {
    let mut remote = connect_scripted(&[
        (
            "SHOW_CONFIGURATION",
            "MODEL=M SERIAL=S FIRMWARE=F CHANNELS=8",
        ),
        ("SHOW_STATUS", "RT=0 LT=0 DT=0% ICR=0 ACTIVE=0 TOTAL=0"),
        ("SHOW_DATA", "DATA 2 0 0"),
        ("START", "ERR the detector is locked by operator"),
    ]);
    let error = remote.start().expect_err("the refusal should surface");
    assert!(
        error.to_string().contains("locked by operator"),
        "unexpected: {error}"
    );
    assert!(!remote.is_active());
}

#[test]
fn polling_refreshes_the_data_at_a_civilised_rate() {
    let mut remote = connect_scripted(&[
        (
            "SHOW_CONFIGURATION",
            "MODEL=M SERIAL=S FIRMWARE=F CHANNELS=4",
        ),
        ("SHOW_STATUS", "RT=0 LT=0 DT=0% ICR=0 ACTIVE=0 TOTAL=0"),
        ("SHOW_DATA", "DATA 4 0 0 0 0"),
        (
            "SHOW_STATUS",
            "RT=2.00 LT=1.90 DT=5.0% ICR=1200 ACTIVE=1 TOTAL=6",
        ),
        ("SHOW_DATA", "DATA 4 1 2 3 0"),
    ]);
    // Well under the poll interval: no traffic.
    remote.poll(0.1).expect("poll");
    // Crossing it: one status, one data.
    remote.poll(0.5).expect("poll");
    assert_eq!(remote.spectrum().channels, vec![1, 2, 3, 0]);
    assert_eq!(remote.status().total_counts, 6);
    assert!((remote.status().live_time - 1.9).abs() < 1e-9);
    assert!(remote.is_active());
}

#[test]
fn a_remote_instrument_and_the_simulator_share_a_pick_list() {
    // The point of AnyMcb: one Vec holds both kinds.
    let simulated: AnyMcb = SimulatedMcb::new(1, "SIM-01").into();
    let remote: AnyMcb = connect_scripted(&[
        (
            "SHOW_CONFIGURATION",
            "MODEL=M SERIAL=S FIRMWARE=F CHANNELS=8",
        ),
        ("SHOW_STATUS", "RT=0 LT=0 DT=0% ICR=0 ACTIVE=0 TOTAL=0"),
        ("SHOW_DATA", "DATA 2 0 0"),
    ])
    .into();
    let mut detectors = [simulated, remote];
    assert_eq!(detectors[0].identity().name, "SIM-01");
    assert_eq!(detectors[1].identity().name, "BENCH-01");
    assert!(detectors[0].as_simulated_mut().is_some());
    assert!(detectors[1].as_simulated_mut().is_none());
}

#[test]
fn optimise_finds_the_pole_zero_this_preamp_needs() {
    let mut mcb = SimulatedMcb::new(1, "TUNE-01");
    // Knock the pole zero well away from wherever the right answer is.
    let mut properties = mcb.properties().clone();
    properties.amplifier.pole_zero = 100;
    mcb.set_properties(properties).expect("set");
    let before = mcb.pole_zero_error().abs();
    assert!(before > 0.05, "the setting should start visibly wrong");

    mcb.start_optimize().expect("start");
    assert!(mcb.optimizing());
    // The routine runs on the instrument's clock, while idle.
    for _ in 0..20 {
        advance(&mut mcb, 1.0).expect("tick");
    }
    assert!(!mcb.optimizing(), "the routine should have finished");
    assert!(
        mcb.pole_zero_error().abs() < 1e-9,
        "optimise should land the pole zero on this preamp's answer"
    );
}

#[test]
fn the_auto_pole_zero_can_be_stopped_early() {
    let mut mcb = SimulatedMcb::new(2, "TUNE-02");
    mcb.set_pole_zero_running(true).expect("start");
    assert!(mcb.pole_zeroing());
    mcb.set_pole_zero_running(false).expect("stop");
    assert!(!mcb.pole_zeroing(), "STOP_PZ should cancel the routine");
}

#[test]
fn tuning_refuses_to_run_over_a_live_count() {
    let mut mcb = SimulatedMcb::new(3, "TUNE-03");
    mcb.start().expect("count");
    assert!(mcb.start_optimize().is_err());
    assert!(mcb.set_pole_zero_running(true).is_err());
}

#[test]
fn the_whole_protocol_round_trips_over_real_tcp() {
    // A simulator served on a real socket; a RemoteMcb connected to it. This is
    // the bench setup with the bench swapped for a thread.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let address = listener.local_addr().expect("address").to_string();
    let served: Arc<Mutex<SimulatedMcb>> = Arc::new(Mutex::new(SimulatedMcb::new(9, "SERVED")));
    let behind = Arc::clone(&served);
    std::thread::spawn(move || {
        if let Ok((stream, _)) = listener.accept() {
            let _ = mantaray_device::server::serve_connection(stream, behind);
        }
    });

    let transport =
        TcpTransport::connect(&address, Duration::from_secs(5)).expect("connect over TCP");
    let mut remote = RemoteMcb::connect(Box::new(transport), 2, "ACROSS-THE-ROOM").expect("hello");
    assert_eq!(
        remote.identity().channels,
        served.lock().unwrap().identity().channels
    );

    // Start the far instrument, let its clock run, and watch the counts arrive
    // on this side of the wire.
    remote.start().expect("start");
    for _ in 0..5 {
        {
            let mut instrument = served.lock().unwrap();
            advance(&mut *instrument, 1.0).expect("advance the far clock");
        }
        remote.poll(1.0).expect("poll");
    }
    assert!(
        remote.spectrum().total_counts() > 0,
        "counts collected over there should be visible over here"
    );
    assert!(remote.status().live_time > 0.0);

    remote.stop().expect("stop");
    assert!(!served.lock().unwrap().is_active(), "the far side stopped");

    // And a raw pass-through, as SEND_MESSAGE uses.
    let version = remote.send_message("SHOW_VERSION").expect("version");
    assert!(!version.is_empty());
}

#[test]
fn an_uncertainty_preset_does_stop_a_remote_instrument() {
    // Worth pinning because it was believed otherwise: the uncertainty and
    // MDA presets are host-side calculations, and no instrument carries them,
    // so it looks as though a bridged or network detector would count on
    // forever. It does not. `advance` is what the desktop application calls
    // for every detector on every frame and what the command line's WAIT
    // calls as it sleeps; it evaluates the presets against the mirrored
    // spectrum and sends STOP over the wire when one is satisfied.
    const PEAK: &str = "DATA 16 5 5 5 10 50 200 400 200 50 10 5 5 5 5 5 5";
    let transport = MockTransport::scripted(&[
        (
            "SHOW_CONFIGURATION",
            "MODEL=M SERIAL=S FIRMWARE=F CHANNELS=16",
        ),
        ("SHOW_STATUS", "RT=0 LT=0 DT=0% ICR=0 ACTIVE=0 TOTAL=0"),
        ("SHOW_DATA", PEAK),
        ("SHOW_PRESETS", "PRESETS REAL=0.00 LIVE=0.00 PEAK=0 INTEG=0"),
        // Only the four the instrument itself carries go over the wire.
        ("SET_PRESET_CLEAR", "OK"),
        ("START", "OK"),
        ("SHOW_STATUS", "RT=10 LT=10 DT=0% ICR=0 ACTIVE=1 TOTAL=960"),
        ("SHOW_DATA", PEAK),
        ("STOP", "OK"),
    ]);
    let mut remote = RemoteMcb::connect(Box::new(transport), 1, "BENCH-01").expect("connect");

    let presets = mantaray_device::Presets {
        uncertainty: Some(mantaray_device::UncertaintyPreset {
            limit_percent: 25.0,
            low_channel: 2,
            high_channel: 12,
        }),
        ..Default::default()
    };
    remote.set_presets(presets).expect("the preset is accepted");
    remote.start().expect("start");
    assert!(remote.is_active());

    let stopped = advance(&mut remote, 2.0).expect("advance");
    assert_eq!(
        stopped,
        Some(mantaray_device::PresetKind::Uncertainty),
        "the uncertainty preset should have stopped the count"
    );
    assert!(!remote.is_active(), "and the instrument should be stopped");
}

#[test]
fn clearing_lets_the_next_start_through_a_preset_that_was_reached() {
    // The regression this pins: CLEAR resets the instrument's clocks, but the
    // mirror kept the old ones until the next poll - and the start guard reads
    // them. With a live preset held, Clear then Start was refused as "already
    // reached", which told the operator to clear the spectrum they had just
    // cleared. A job's CLEAR / START did not start at all.
    let mut remote = connect_exactly(&[
        (
            "SHOW_CONFIGURATION",
            "MODEL=M SERIAL=S FIRMWARE=F CHANNELS=8",
        ),
        // Counted out: the live preset below is already satisfied.
        (
            "SHOW_STATUS",
            "RT=310.90 LT=300.00 DT=3.51% ICR=0 ACTIVE=0 TOTAL=0",
        ),
        ("SHOW_DATA", "DATA 2 0 0"),
        (
            "SHOW_PRESETS",
            "PRESETS REAL=0.00 LIVE=300.00 PEAK=0 INTEG=0",
        ),
        // Exactly what the operator does next, and nothing in between.
        ("CLEAR", "OK"),
        ("START", "OK"),
    ]);
    assert_eq!(remote.presets().live_time, Some(300.0));
    assert!(
        remote.start().is_err(),
        "before clearing, the reached preset is what refuses"
    );

    remote.clear().expect("clear");
    assert_eq!(
        remote.status().live_time,
        0.0,
        "the mirror's clocks go with the instrument's"
    );
    remote
        .start()
        .expect("after clearing, the preset is no longer reached");
    assert!(remote.is_active());
}

/// A transport whose fetches wait at a gate the test holds shut.
///
/// The connection handshake answers immediately, like any instrument; after
/// it, every `SHOW_STATUS` stands at the gate until the test lets it through.
/// This is the slow instrument from the bench, distilled: the 325 ms it took
/// to answer becomes "as long as the test likes".
struct Sluggish {
    handshake: std::collections::VecDeque<(&'static str, &'static str)>,
    gate: std::sync::mpsc::Receiver<()>,
}

impl mantaray_device::Transport for Sluggish {
    fn exchange(&mut self, command: &str) -> Result<String, mantaray_device::DeviceError> {
        if let Some((expected, reply)) = self.handshake.pop_front() {
            assert_eq!(command, expected, "the handshake is out of order");
            return Ok(reply.to_string());
        }
        Ok(match command {
            "SHOW_STATUS" => {
                // The instrument taking its time, for exactly as long as the
                // test wants it to.
                self.gate.recv().expect("the test holds the gate's sender");
                "RT=9.00 LT=8.00 DT=1.0% ICR=0 ACTIVE=1 TOTAL=8".to_string()
            }
            "SHOW_DATA" => "DATA 4 2 2 2 2".to_string(),
            _ => "OK".to_string(),
        })
    }

    fn peer(&self) -> String {
        "sluggish".to_string()
    }
}

/// Behind a courier, a poll returns before a slow instrument answers.
///
/// This is the Dell bench finding, pinned: a fetch took 325 ms on the
/// instrument's side and ran on the interface's thread, so every other frame
/// froze for a third of a second. With the fetch in a courier's hands the
/// poll must come back at once - here, while the instrument is still standing
/// at a gate that has not opened - and the numbers arrive on a later poll,
/// once the instrument has actually answered.
#[test]
fn a_poll_over_a_courier_returns_before_a_slow_instrument_answers() {
    let (open_gate, gate) = std::sync::mpsc::channel();
    let transport = Sluggish {
        handshake: [
            (
                "SHOW_CONFIGURATION",
                "MODEL=SLOW-1 SERIAL=1 FIRMWARE=v1 CHANNELS=4",
            ),
            (
                "SHOW_STATUS",
                "RT=0.00 LT=0.00 DT=0.0% ICR=0 ACTIVE=1 TOTAL=0",
            ),
            ("SHOW_DATA", "DATA 4 0 0 0 0"),
            ("SHOW_PRESETS", "PRESETS REAL=0 LIVE=0 PEAK=0 INTEG=0"),
        ]
        .into_iter()
        .collect(),
        gate,
    };
    let mut remote = RemoteMcb::connect(Box::new(transport), 1, "SLOW")
        .expect("connect")
        .with_courier();

    // The first poll past the interval asks for a fetch. The instrument now
    // stands at the gate - and the poll has already returned, which under the
    // old arrangement is exactly what could not happen.
    remote.poll(1.0).expect("poll");
    assert_eq!(
        remote.status().total_counts,
        0,
        "nothing can have arrived: the instrument has not answered yet"
    );

    // Further polls keep coming back without waiting, and without piling a
    // queue of fetches behind the one still standing at the gate.
    for _ in 0..3 {
        remote.poll(1.0).expect("poll while the instrument dawdles");
    }
    assert_eq!(remote.status().total_counts, 0);

    // Let the instrument answer, and the numbers arrive on a later poll.
    open_gate.send(()).expect("open the gate");
    let arrived = std::time::Instant::now();
    loop {
        remote.poll(0.0).expect("poll");
        if remote.status().total_counts == 8 {
            break;
        }
        assert!(
            arrived.elapsed() < Duration::from_secs(5),
            "the released fetch never arrived"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(remote.spectrum().channels, vec![2, 2, 2, 2]);
    assert!((remote.status().real_time - 9.0).abs() < 1e-9);

    // Commands still round-trip through the same line, in order.
    remote.stop().expect("STOP answers over the courier");
}

/// An instrument with a before and an after: CLEAR moves it between eras.
///
/// Fetches stand at the gate like [`Sluggish`]'s; the answers carry the
/// pre-clear count until CLEAR arrives and the post-clear one afterwards, so
/// a test can tell exactly which era a collected fetch came from.
struct TwoEra {
    handshake: std::collections::VecDeque<(&'static str, &'static str)>,
    gate: std::sync::mpsc::Receiver<()>,
    cleared: bool,
}

impl mantaray_device::Transport for TwoEra {
    fn exchange(&mut self, command: &str) -> Result<String, mantaray_device::DeviceError> {
        if let Some((expected, reply)) = self.handshake.pop_front() {
            assert_eq!(command, expected, "the handshake is out of order");
            return Ok(reply.to_string());
        }
        Ok(match command {
            "SHOW_STATUS" => {
                self.gate.recv().expect("the test holds the gate's sender");
                if self.cleared {
                    "RT=1.50 LT=1.40 DT=1.0% ICR=0 ACTIVE=1 TOTAL=4".to_string()
                } else {
                    "RT=9.00 LT=8.00 DT=1.0% ICR=0 ACTIVE=1 TOTAL=8".to_string()
                }
            }
            "SHOW_DATA" if self.cleared => "DATA 4 1 1 1 1".to_string(),
            "SHOW_DATA" => "DATA 4 2 2 2 2".to_string(),
            "CLEAR" => {
                self.cleared = true;
                "OK".to_string()
            }
            _ => "OK".to_string(),
        })
    }

    fn peer(&self) -> String {
        "two-era".to_string()
    }
}

/// A fetch from before a CLEAR is discarded, never integrated after it.
///
/// The courier runs errands in order, so a fetch requested before a command
/// has always come back by the time the command answers - describing the
/// instrument from before it. Collected after a CLEAR, it put the
/// thrown-away spectrum straight back on screen, and worse: CLEAR empties
/// the mirror's start date, the stale status still said active-with-real-time,
/// and the start-of-run reconstruction planted a date one whole discarded run
/// in the past - which then stuck, because a date is only ever reconstructed
/// into a gap. On the bench that prompted the courier, a fetch is in flight
/// two-thirds of the time, so most CLEARs raced one.
#[test]
fn a_fetch_from_before_a_clear_is_discarded_rather_than_integrated() {
    let (open_gate, gate) = std::sync::mpsc::channel();
    let transport = TwoEra {
        handshake: [
            (
                "SHOW_CONFIGURATION",
                "MODEL=SLOW-1 SERIAL=1 FIRMWARE=v1 CHANNELS=4",
            ),
            (
                "SHOW_STATUS",
                "RT=0.00 LT=0.00 DT=0.0% ICR=0 ACTIVE=0 TOTAL=0",
            ),
            ("SHOW_DATA", "DATA 4 0 0 0 0"),
            ("SHOW_PRESETS", "PRESETS REAL=0 LIVE=0 PEAK=0 INTEG=0"),
        ]
        .into_iter()
        .collect(),
        gate,
        cleared: false,
    };
    let mut remote = RemoteMcb::connect(Box::new(transport), 1, "SLOW")
        .expect("connect")
        .with_courier();

    // A fetch goes out and completes: pre-clear counts, pre-clear clocks.
    remote.poll(1.0).expect("poll");
    open_gate.send(()).expect("open the gate");

    // CLEAR is queued behind it, so when this returns, that fetch is already
    // lying in the slot - and must have been thrown away, not left waiting.
    remote.clear().expect("clear");
    assert_eq!(remote.spectrum().total_counts(), 0);
    assert_eq!(remote.spectrum().start_time, None);

    // The next poll collects nothing: the cleared spectrum stays cleared,
    // and no start date is reconstructed from the discarded run's clock.
    remote.poll(0.0).expect("poll");
    assert_eq!(
        remote.spectrum().total_counts(),
        0,
        "a fetch from before the CLEAR was integrated after it"
    );
    assert_eq!(
        remote.spectrum().start_time,
        None,
        "a start date was reconstructed from the discarded run's clock"
    );

    // Fetching then resumes, and the next fetch - from after the CLEAR -
    // arrives whole, with a start date belonging to the run that is real.
    remote.poll(1.0).expect("poll");
    open_gate.send(()).expect("open the gate again");
    let waited = std::time::Instant::now();
    while remote.status().total_counts != 4 {
        assert!(
            waited.elapsed() < Duration::from_secs(5),
            "the post-clear fetch never arrived"
        );
        std::thread::sleep(Duration::from_millis(2));
        remote.poll(0.0).expect("poll");
    }
    assert_eq!(remote.spectrum().channels, vec![1, 1, 1, 1]);
    assert!((remote.status().real_time - 1.5).abs() < 1e-9);
    assert!(
        remote.spectrum().start_time.is_some(),
        "the real run's start is reconstructed as before"
    );
}

/// A transport that dies on the first fetch, taking the courier's thread.
///
/// The panic it raises is printed by the test harness; it is this test's
/// doing, not a failure.
struct Doomed {
    handshake: std::collections::VecDeque<(&'static str, &'static str)>,
}

impl mantaray_device::Transport for Doomed {
    fn exchange(&mut self, command: &str) -> Result<String, mantaray_device::DeviceError> {
        if let Some((expected, reply)) = self.handshake.pop_front() {
            assert_eq!(command, expected, "the handshake is out of order");
            return Ok(reply.to_string());
        }
        panic!("the transport gave out");
    }

    fn peer(&self) -> String {
        "doomed".to_string()
    }
}

/// A courier whose thread has died says so, rather than standing forever.
///
/// A panic in the transport ends the courier's thread mid-fetch. Without
/// noticing, the poll would keep returning cleanly with nothing collected -
/// no error, no data, a mirror frozen on numbers that will never change,
/// reading exactly like a healthy idle instrument. It must read as what it
/// is: a lost connection.
#[test]
fn a_courier_whose_thread_died_reports_a_lost_connection() {
    let transport = Doomed {
        handshake: [
            (
                "SHOW_CONFIGURATION",
                "MODEL=SLOW-1 SERIAL=1 FIRMWARE=v1 CHANNELS=4",
            ),
            (
                "SHOW_STATUS",
                "RT=0.00 LT=0.00 DT=0.0% ICR=0 ACTIVE=0 TOTAL=0",
            ),
            ("SHOW_DATA", "DATA 4 0 0 0 0"),
            ("SHOW_PRESETS", "PRESETS REAL=0 LIVE=0 PEAK=0 INTEG=0"),
        ]
        .into_iter()
        .collect(),
    };
    let mut remote = RemoteMcb::connect(Box::new(transport), 1, "SLOW")
        .expect("connect")
        .with_courier();

    // The first poll past the interval asks for the fetch that kills the
    // thread; a later poll must come back with the loss, not with silence.
    let waited = std::time::Instant::now();
    let error = loop {
        if let Err(error) = remote.poll(1.0) {
            break error;
        }
        assert!(
            waited.elapsed() < Duration::from_secs(5),
            "the dead courier was never reported"
        );
        std::thread::sleep(Duration::from_millis(2));
    };
    assert!(
        matches!(error, mantaray_device::DeviceError::Connection { .. }),
        "a dead courier is a lost connection: {error}"
    );
}

/// A channel count that flaps does not clear the operator's spectrum.
///
/// On the Windows road, ORTEC's library is asked the detector's length on
/// every read and truncates the data to what it actually returned - and a
/// busy instrument mid-acquisition answers short. Rebuilding the mirror for
/// each disagreement started it from zeros, so the window flashed empty and
/// full, over and over: seen on the bench as the program clearing the count.
/// A mirror holding counts now believes a new length only when two fetches
/// in a row agree on it; the flap costs its own channels and nothing else -
/// its clocks still land.
#[test]
fn a_flapping_channel_count_does_not_clear_the_mirror() {
    let mut remote = connect_scripted(&[
        (
            "SHOW_CONFIGURATION",
            "MODEL=MCS-1 SERIAL=7 FIRMWARE=v1 CHANNELS=4",
        ),
        (
            "SHOW_STATUS",
            "RT=1.00 LT=1.00 DT=0.0% ICR=0 ACTIVE=1 TOTAL=20",
        ),
        ("SHOW_DATA", "DATA 4 5 5 5 5"),
        // A short read: two channels of a four-channel instrument.
        (
            "SHOW_STATUS",
            "RT=2.00 LT=2.00 DT=0.0% ICR=0 ACTIVE=1 TOTAL=18",
        ),
        ("SHOW_DATA", "DATA 2 9 9"),
        // The next read is whole again, and its counts must land.
        (
            "SHOW_STATUS",
            "RT=3.00 LT=3.00 DT=0.0% ICR=0 ACTIVE=1 TOTAL=24",
        ),
        ("SHOW_DATA", "DATA 4 6 6 6 6"),
    ]);
    assert_eq!(remote.spectrum().channels, vec![5, 5, 5, 5]);

    // The flap: the mirror keeps its shape and its counts...
    remote.poll(1.0).expect("poll through the short read");
    assert_eq!(
        remote.spectrum().channels,
        vec![5, 5, 5, 5],
        "a one-fetch flap must not rebuild the mirror"
    );
    // ...while the flapped fetch's clocks still count.
    assert!((remote.status().real_time - 2.0).abs() < 1e-9);

    // The whole read after it lands as if nothing happened.
    remote.poll(1.0).expect("poll through the whole read");
    assert_eq!(remote.spectrum().channels, vec![6, 6, 6, 6]);
}

/// A length change that repeats is real, and is adopted - once.
///
/// A genuine conversion-gain change mid-run answers with the new length on
/// every fetch from then on, so the second agreeing fetch is what rebuilds
/// the mirror; an empty mirror (connection, the fetch after CLEAR) adopts at
/// once, because rebuilding zeros as zeros loses nothing.
#[test]
fn a_persistent_length_change_is_adopted_on_the_second_fetch() {
    let mut remote = connect_scripted(&[
        (
            "SHOW_CONFIGURATION",
            "MODEL=MCS-1 SERIAL=7 FIRMWARE=v1 CHANNELS=4",
        ),
        (
            "SHOW_STATUS",
            "RT=1.00 LT=1.00 DT=0.0% ICR=0 ACTIVE=1 TOTAL=20",
        ),
        ("SHOW_DATA", "DATA 4 5 5 5 5"),
        (
            "SHOW_STATUS",
            "RT=2.00 LT=2.00 DT=0.0% ICR=0 ACTIVE=1 TOTAL=18",
        ),
        ("SHOW_DATA", "DATA 2 9 9"),
        (
            "SHOW_STATUS",
            "RT=3.00 LT=3.00 DT=0.0% ICR=0 ACTIVE=1 TOTAL=18",
        ),
        ("SHOW_DATA", "DATA 2 9 9"),
    ]);

    // First sighting: not yet believed.
    remote.poll(1.0).expect("first fetch at the new length");
    assert_eq!(remote.spectrum().channels, vec![5, 5, 5, 5]);

    // Second in a row: the change is real, the mirror follows it.
    remote.poll(1.0).expect("second fetch at the new length");
    assert_eq!(
        remote.spectrum().channels,
        vec![9, 9],
        "two agreeing fetches are a real gain change"
    );
}
