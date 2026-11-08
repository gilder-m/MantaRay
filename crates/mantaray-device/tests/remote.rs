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
    let mut remote = RemoteMcb::connect(Box::new(transport), 7, "BENCH-01").expect("connect");
    remote.set_confirm_step(Duration::ZERO);
    remote
}

/// The status read before a START (no held preset is reached) or after a
/// STOP (it has stopped).
const IDLE: (&str, &str) = ("SHOW_STATUS", "RT=0 LT=0 DT=0% ICR=0 ACTIVE=0 TOTAL=0");

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
        // The fresh reading Start takes; no START is scripted: sending one
        // would be the bug.
        (
            "SHOW_STATUS",
            "RT=310.90 LT=300.00 DT=3.51% ICR=0 ACTIVE=0 TOTAL=0",
        ),
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
fn starting_reads_the_clocks_and_then_sends_the_start_command() {
    let mut remote = connect_scripted(&[
        (
            "SHOW_CONFIGURATION",
            "MODEL=M SERIAL=S FIRMWARE=F CHANNELS=8",
        ),
        ("SHOW_STATUS", "RT=0 LT=0 DT=0% ICR=0 ACTIVE=0 TOTAL=0"),
        ("SHOW_DATA", "DATA 2 0 0"),
        IDLE,
        ("START", "OK"),
    ]);
    remote.start().expect("start");
    assert!(
        !remote.is_active(),
        "the mirror says counting only once a reading does"
    );
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
        IDLE,
        ("START", "OK"),
        ("STOP", "OK"),
        IDLE,
        IDLE,
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
        ("SHOW_STATUS", "RT=0 LT=0 DT=0% ICR=0 ACTIVE=0 TOTAL=0"),
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
    // The instrument's own refusal, at once: no read after it.
    let script = vec![
        (
            "SHOW_CONFIGURATION",
            "MODEL=M SERIAL=S FIRMWARE=F CHANNELS=8",
        ),
        ("SHOW_STATUS", "RT=0 LT=0 DT=0% ICR=0 ACTIVE=0 TOTAL=0"),
        ("SHOW_DATA", "DATA 2 0 0"),
        IDLE,
        ("START", "ERR the detector is locked by operator"),
    ];
    let mut remote = connect_scripted(&script);
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
        // Only the four the instrument itself carries go over the wire, and
        // an uncertainty preset alone is not one: nothing at all is sent.
        ("SHOW_STATUS", "RT=0 LT=0 DT=0% ICR=0 ACTIVE=0 TOTAL=0"),
        IDLE,
        ("START", "OK"),
        ("SHOW_STATUS", "RT=10 LT=10 DT=0% ICR=0 ACTIVE=1 TOTAL=960"),
        ("SHOW_DATA", PEAK),
        ("STOP", "OK"),
        IDLE,
    ]);
    let mut remote = RemoteMcb::connect(Box::new(transport), 1, "BENCH-01").expect("connect");

    // A host-side preset alone: nothing is sent for it.
    remote
        .apply_presets(mantaray_device::Presets {
            uncertainty: Some(mantaray_device::UncertaintyPreset {
                limit_percent: 25.0,
                low_channel: 2,
                high_channel: 12,
            }),
            ..Default::default()
        })
        .expect("the preset is accepted");
    remote.start().expect("start");

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
        // Exactly what the operator does next: Start (refused on the fresh
        // reading, nothing sent), Clear, Start (the fresh reading, then
        // START).
        (
            "SHOW_STATUS",
            "RT=310.90 LT=300.00 DT=3.51% ICR=0 ACTIVE=0 TOTAL=0",
        ),
        ("CLEAR", "OK"),
        IDLE,
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

    // Commands still round-trip through the same line, in order. (CLEAR: a
    // STOP is confirmed by status reads, and this instrument's stand at the
    // gate.)
    remote.clear().expect("CLEAR answers over the courier");
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

/// A line that answers like a quiet eight-channel instrument and keeps a log
/// of every command sent, for tests that care what did *not* go out.
struct Recording {
    log: Arc<Mutex<Vec<String>>>,
}

impl mantaray_device::Transport for Recording {
    fn exchange(&mut self, command: &str) -> Result<String, mantaray_device::DeviceError> {
        self.log.lock().expect("log").push(command.to_string());
        Ok(match command {
            "SHOW_CONFIGURATION" => "MODEL=DSPEC-50 SERIAL=1 FIRMWARE=1 CHANNELS=8".into(),
            "SHOW_STATUS" => "RT=0.00 LT=0.00 DT=0.0% ICR=0 ACTIVE=0 TOTAL=0".into(),
            "SHOW_DATA" => "DATA 8 0 0 0 0 0 0 0 0".into(),
            "SHOW_PRESETS" => "PRESETS REAL=0.00 LIVE=300.00 PEAK=0 INTEG=0".into(),
            _ => "OK".into(),
        })
    }

    fn peer(&self) -> String {
        "recording".into()
    }
}

fn connect_recording() -> (RemoteMcb, Arc<Mutex<Vec<String>>>) {
    let log = Arc::new(Mutex::new(Vec::new()));
    let transport = Recording { log: log.clone() };
    let remote = RemoteMcb::connect(Box::new(transport), 1, "BENCH").expect("connect");
    log.lock().expect("log").clear();
    (remote, log)
}

#[test]
fn editing_one_setting_sends_only_that_setting() {
    // The mirror's bias is a default, not a reading. Before this, changing the
    // lower discriminator sent SET_HV 3000, SET_GAIN_COARSE 20 and
    // SET_PRESET_CLEAR: a bias nobody chose, and the presets of the count
    // gone.
    let (mut remote, log) = connect_recording();
    let mut properties = remote.properties().clone();
    properties.adc.lower_discriminator = 10;
    remote.set_properties(properties).expect("set");
    assert_eq!(*log.lock().expect("log"), Vec::<String>::new());
    assert_eq!(remote.properties().adc.lower_discriminator, 10);
    assert_eq!(
        remote.presets().live_time,
        Some(300.0),
        "the instrument's preset is still mirrored"
    );
}

#[test]
fn the_bias_and_the_gain_go_out_only_when_they_change() {
    let (mut remote, log) = connect_recording();
    let mut properties = remote.properties().clone();
    properties.high_voltage.target_volts = 1500.0;
    remote.set_properties(properties.clone()).expect("bias");
    assert_eq!(*log.lock().expect("log"), vec!["SET_HV 1500".to_string()]);

    log.lock().expect("log").clear();
    properties.amplifier.coarse_gain = 50.0;
    remote.set_properties(properties).expect("gain");
    assert_eq!(
        *log.lock().expect("log"),
        vec!["SET_GAIN_COARSE 50".to_string()],
        "the bias already set is not sent again"
    );
    assert_eq!(remote.properties().high_voltage.target_volts, 1500.0);
}

#[test]
fn switching_the_bias_off_is_refused_rather_than_faked() {
    // Unticking Enabled used to send SET_HV with the target - bias left on -
    // while the dialog showed it off.
    let (mut remote, log) = connect_recording();
    let before = remote.properties().high_voltage.enabled;
    let mut properties = remote.properties().clone();
    properties.high_voltage.enabled = !before;
    let error = remote
        .set_properties(properties)
        .expect_err("no on/off command is proven");
    assert!(error.to_string().contains("high voltage"), "{error}");
    assert_eq!(*log.lock().expect("log"), Vec::<String>::new());
    assert_eq!(
        remote.properties().high_voltage.enabled,
        before,
        "the mirror does not claim the change"
    );
}

#[test]
fn the_upper_discriminator_starts_inside_the_instrument() {
    // At u32::MAX the ADC tab's range clamp fired on first draw, and the
    // clamp counted as an edit.
    let (remote, _) = connect_recording();
    assert_eq!(remote.properties().adc.upper_discriminator, 7);
}

#[test]
fn an_applied_bias_is_sent_even_when_the_mirror_already_says_it() {
    // The mirror's target is a default, not a reading: applying that same
    // number must still reach the instrument, or 3000 V could never be set.
    let (mut remote, log) = connect_recording();
    let shown = remote.properties().high_voltage.target_volts;
    remote.set_high_voltage(shown).expect("bias");
    assert_eq!(
        *log.lock().expect("log"),
        vec![format!("SET_HV {shown:.0}")]
    );

    log.lock().expect("log").clear();
    let mut any: mantaray_device::AnyMcb = remote.into();
    any.set_high_voltage(1200.0).expect("through the pick list");
    assert_eq!(*log.lock().expect("log"), vec!["SET_HV 1200".to_string()]);
    assert_eq!(any.properties().high_voltage.target_volts, 1200.0);
}

#[test]
fn the_simulator_takes_an_applied_bias_too() {
    let mut any: AnyMcb = SimulatedMcb::new(1, "SIM-01").into();
    any.set_high_voltage(1234.0).expect("bias");
    assert_eq!(any.properties().high_voltage.target_volts, 1234.0);
}

#[test]
fn an_applied_coarse_gain_is_sent_even_when_the_mirror_already_says_it() {
    // The mirror's 20 is a default, not a reading: applying that same number
    // must still reach the instrument, as the bias does.
    let (mut remote, log) = connect_recording();
    let shown = remote.properties().amplifier.coarse_gain;
    remote.set_coarse_gain(shown).expect("gain");
    assert_eq!(
        *log.lock().expect("log"),
        vec![format!("SET_GAIN_COARSE {shown:.0}")]
    );

    log.lock().expect("log").clear();
    let mut any: mantaray_device::AnyMcb = remote.into();
    any.set_coarse_gain(40.0).expect("through the pick list");
    any.set_coarse_gain(40.0).expect("and again, unchanged");
    assert_eq!(
        *log.lock().expect("log"),
        vec!["SET_GAIN_COARSE 40".to_string(); 2]
    );
    assert_eq!(any.properties().amplifier.coarse_gain, 40.0);
}

#[test]
fn the_simulator_takes_an_applied_coarse_gain_too() {
    let mut any: AnyMcb = SimulatedMcb::new(1, "SIM-01").into();
    any.set_coarse_gain(64.0).expect("gain");
    assert_eq!(any.properties().amplifier.coarse_gain, 64.0);
}

#[test]
fn the_mirror_holds_exactly_what_was_sent() {
    // 20.4 went out as "SET_GAIN_COARSE 20" and 1234.5 as "SET_HV 1234" while
    // the mirror, and so the dialog, kept the fraction. Whole numbers only,
    // so there is nothing to round.
    let (mut remote, log) = connect_recording();
    let mut properties = remote.properties().clone();
    properties.amplifier.coarse_gain = 21.0;
    properties.high_voltage.target_volts = 1234.0;
    remote.set_properties(properties).expect("set");
    assert_eq!(
        *log.lock().expect("log"),
        vec!["SET_HV 1234".to_string(), "SET_GAIN_COARSE 21".to_string()]
    );
    assert_eq!(remote.properties().amplifier.coarse_gain, 21.0);
    assert_eq!(remote.properties().high_voltage.target_volts, 1234.0);
}

#[test]
fn a_gain_or_bias_that_is_not_a_whole_number_in_range_is_refused_before_it_is_sent() {
    let (mut remote, log) = connect_recording();
    let before = remote.properties().clone();
    for bad in [f64::NAN, f64::INFINITY, 0.0, -5.0, 20.4, 1001.0, 1e9] {
        let mut properties = before.clone();
        properties.amplifier.coarse_gain = bad;
        assert!(remote.set_properties(properties).is_err(), "gain {bad}");
        assert!(remote.set_coarse_gain(bad).is_err(), "gain {bad}");
    }
    for bad in [f64::NAN, f64::NEG_INFINITY, -1.0, 1500.5, 5001.0] {
        let mut properties = before.clone();
        properties.high_voltage.target_volts = bad;
        assert!(remote.set_properties(properties).is_err(), "bias {bad}");
        assert!(remote.set_high_voltage(bad).is_err(), "bias {bad}");
    }
    assert_eq!(*log.lock().expect("log"), Vec::<String>::new());
    assert_eq!(remote.properties(), &before);
    // The edges of the range are fine.
    for good in [1.0, 1000.0] {
        remote.set_coarse_gain(good).expect("gain");
    }
    for good in [0.0, 5000.0] {
        remote.set_high_voltage(good).expect("bias");
    }
}

#[test]
fn the_simulator_refuses_what_the_network_instrument_refuses() {
    // One check, used by both, on every way in: the properties and the wire.
    let (mut remote, _) = connect_recording();
    let mut sim = SimulatedMcb::new(1, "SIM-01");
    for bad in [f64::NAN, 0.0, 20.4, 1001.0] {
        let mut properties = sim.properties().clone();
        properties.amplifier.coarse_gain = bad;
        assert!(sim.set_properties(properties).is_err(), "gain {bad}");
        assert!(remote.set_coarse_gain(bad).is_err(), "gain {bad}");
        if bad.is_finite() {
            assert!(
                sim.send_message(&format!("SET_GAIN_COARSE {bad}")).is_err(),
                "wire gain {bad}"
            );
        }
    }
    for bad in [f64::NAN, -1.0, 1500.5, 5001.0] {
        let mut properties = sim.properties().clone();
        properties.high_voltage.target_volts = bad;
        assert!(sim.set_properties(properties).is_err(), "bias {bad}");
        assert!(remote.set_high_voltage(bad).is_err(), "bias {bad}");
        if bad.is_finite() {
            assert!(
                sim.send_message(&format!("SET_HV {bad}")).is_err(),
                "wire bias {bad}"
            );
        }
    }
    assert_eq!(sim.properties().amplifier.coarse_gain, 20.0);
    assert_eq!(sim.properties().high_voltage.target_volts, 3000.0);
    // And the same good values go through both.
    sim.send_message("SET_GAIN_COARSE 50").expect("wire gain");
    sim.send_message("SET_HV 1200").expect("wire bias");
    assert_eq!(sim.properties().amplifier.coarse_gain, 50.0);
    assert_eq!(sim.properties().high_voltage.target_volts, 1200.0);
}

#[test]
fn negative_zero_is_sent_and_kept_as_zero() {
    // `{:.0}` prints -0.0 as "-0". A bias of 0 V typed as -0 must go out as 0.
    let (mut remote, log) = connect_recording();
    remote.set_high_voltage(-0.0).expect("bias");
    assert_eq!(*log.lock().expect("log"), vec!["SET_HV 0".to_string()]);
    assert!(
        remote
            .properties()
            .high_voltage
            .target_volts
            .is_sign_positive()
    );

    log.lock().expect("log").clear();
    let mut properties = remote.properties().clone();
    properties.high_voltage.target_volts = 1200.0;
    remote.set_properties(properties).expect("set");
    let mut properties = remote.properties().clone();
    properties.high_voltage.target_volts = -0.0;
    remote.set_properties(properties).expect("set");
    assert_eq!(
        *log.lock().expect("log"),
        vec!["SET_HV 1200".to_string(), "SET_HV 0".to_string()]
    );
    assert!(
        remote
            .properties()
            .high_voltage
            .target_volts
            .is_sign_positive()
    );

    let mut sim = SimulatedMcb::new(1, "SIM-01");
    sim.send_message("SET_HV -0").expect("wire bias");
    assert!(
        sim.properties()
            .high_voltage
            .target_volts
            .is_sign_positive()
    );
    sim.set_high_voltage(-0.0).expect("bias");
    assert!(
        sim.properties()
            .high_voltage
            .target_volts
            .is_sign_positive()
    );
}

fn an_mda_preset() -> mantaray_device::MdaPreset {
    mantaray_device::MdaPreset {
        target: 10.0,
        region: mantaray_core::Roi::new(1, 4),
        efficiency: 0.05,
        yield_fraction: 0.85,
    }
}

#[test]
fn host_side_presets_never_reach_the_instrument() {
    // The uncertainty and MDA presets are worked out on this side. Changing
    // them sent SET_PRESET_CLEAR and the presets again, wiping the
    // instrument's own presets for an edit it never held. (While the detector
    // counts no preset changes at all: see
    // `every_preset_road_is_refused_while_the_instrument_counts`.)
    let (mut remote, log) = connect_recording();

    remote
        .apply_presets(mantaray_device::Presets {
            mda: Some(an_mda_preset()),
            uncertainty: Some(mantaray_device::UncertaintyPreset {
                limit_percent: 1.0,
                low_channel: 0,
                high_channel: 3,
            }),
            ..*remote.presets()
        })
        .expect("host-side edit");
    // Only the status read goes to the instrument: no command changes it.
    assert_eq!(*log.lock().expect("log"), vec!["SHOW_STATUS".to_string()]);
    log.lock().expect("log").clear();
    assert_eq!(remote.presets().mda, Some(an_mda_preset()));
    assert_eq!(remote.presets().live_time, Some(300.0), "still mirrored");

    // The instrument's own presets do go, and exactly those.
    remote
        .apply_presets(mantaray_device::Presets {
            live_time: Some(60.0),
            ..*remote.presets()
        })
        .expect("live preset");
    assert_eq!(
        *log.lock().expect("log"),
        vec![
            "SHOW_STATUS".to_string(),
            "SET_PRESET_CLEAR".to_string(),
            "SET_PRESET_LIVE 60".to_string()
        ]
    );
    assert_eq!(remote.presets().live_time, Some(60.0));
    assert_eq!(remote.presets().mda, Some(an_mda_preset()));
}

/// A line like [`Recording`] that refuses the commands whose names it is given.
struct Refusing {
    refuse: Vec<&'static str>,
}

impl mantaray_device::Transport for Refusing {
    fn exchange(&mut self, command: &str) -> Result<String, mantaray_device::DeviceError> {
        if self.refuse.iter().any(|name| command.starts_with(name)) {
            return Ok("ERR not allowed now".into());
        }
        Ok(match command {
            "SHOW_CONFIGURATION" => "MODEL=DSPEC-50 SERIAL=1 FIRMWARE=1 CHANNELS=8".into(),
            "SHOW_STATUS" => "RT=0.00 LT=0.00 DT=0.0% ICR=0 ACTIVE=0 TOTAL=0".into(),
            "SHOW_DATA" => "DATA 8 0 0 0 0 0 0 0 0".into(),
            "SHOW_PRESETS" => "PRESETS REAL=0.00 LIVE=300.00 PEAK=0 INTEG=0".into(),
            _ => "OK".into(),
        })
    }

    fn peer(&self) -> String {
        "refusing".into()
    }
}

#[test]
fn a_partial_send_says_what_was_applied_and_the_mirror_agrees() {
    let mut remote = RemoteMcb::connect(
        Box::new(Refusing {
            refuse: vec!["SET_GAIN_COARSE"],
        }),
        1,
        "BENCH",
    )
    .expect("connect");
    let before = remote.properties().amplifier.coarse_gain;
    let mut properties = remote.properties().clone();
    properties.high_voltage.target_volts = 1200.0;
    properties.adc.lower_discriminator = 10;
    properties.amplifier.coarse_gain = before * 2.0;
    let error = remote
        .set_properties(properties)
        .expect_err("the gain is refused");
    let text = error.to_string();
    assert!(text.contains("bias set to 1200 V"), "{text}");
    assert!(text.contains("coarse gain not set"), "{text}");
    assert!(text.contains("not allowed now"), "{text}");
    // The bias went out and is mirrored; the edit with no command is kept; the
    // gain the instrument still holds is what the mirror says.
    assert_eq!(remote.properties().high_voltage.target_volts, 1200.0);
    assert_eq!(remote.properties().adc.lower_discriminator, 10);
    assert_eq!(remote.properties().amplifier.coarse_gain, before);
}

#[test]
fn a_refusal_before_anything_went_out_is_the_instruments_own_error() {
    let mut remote = RemoteMcb::connect(
        Box::new(Refusing {
            refuse: vec!["SET_HV"],
        }),
        1,
        "BENCH",
    )
    .expect("connect");
    let mut properties = remote.properties().clone();
    properties.high_voltage.target_volts = 1200.0;
    properties.amplifier.coarse_gain *= 2.0;
    let error = remote.set_properties(properties).expect_err("refused");
    let text = error.to_string();
    assert!(!text.contains("set to"), "nothing was applied: {text}");
    assert!(text.contains("not allowed now"), "{text}");
    assert_ne!(remote.properties().high_voltage.target_volts, 1200.0);
}

#[test]
fn presets_cleared_and_then_refused_leave_the_mirror_as_the_instrument_is() {
    // SET_PRESET_CLEAR went out, SET_PRESET_LIVE was refused: the instrument
    // holds no presets now, and the mirror must not keep claiming the old one.
    let mut remote = RemoteMcb::connect(
        Box::new(Refusing {
            refuse: vec!["SET_PRESET_LIVE"],
        }),
        1,
        "BENCH",
    )
    .expect("connect");
    let error = remote
        .set_presets(mantaray_device::Presets::live(60.0))
        .expect_err("refused");
    let text = error.to_string();
    assert!(text.contains("presets cleared"), "{text}");
    assert!(text.contains("live-time preset not set"), "{text}");
    assert_eq!(remote.presets().live_time, None);
}

/// A [`Recording`] line whose instrument has been counting a peak, so an
/// uncertainty preset can be satisfied, and which refuses what it is told to.
struct CountingPeak {
    refuse: Vec<&'static str>,
    log: Arc<Mutex<Vec<String>>>,
    /// Starts counting; STOP and START move it, as an instrument's would.
    counting: bool,
}

impl mantaray_device::Transport for CountingPeak {
    fn exchange(&mut self, command: &str) -> Result<String, mantaray_device::DeviceError> {
        self.log.lock().expect("log").push(command.to_string());
        if self.refuse.iter().any(|name| command.starts_with(name)) {
            return Ok("ERR not allowed now".into());
        }
        match command {
            "STOP" => self.counting = false,
            "START" => self.counting = true,
            _ => {}
        }
        Ok(match command {
            "SHOW_CONFIGURATION" => "MODEL=M SERIAL=S FIRMWARE=F CHANNELS=16".into(),
            "SHOW_STATUS" => format!(
                "RT=10 LT=10 DT=0% ICR=0 ACTIVE={} TOTAL=960",
                u8::from(self.counting)
            ),
            "SHOW_DATA" => "DATA 16 5 5 5 10 50 200 400 200 50 10 5 5 5 5 5 5".into(),
            "SHOW_PRESETS" => "PRESETS REAL=0.00 LIVE=300.00 PEAK=0 INTEG=0".into(),
            _ => "OK".into(),
        })
    }

    fn peer(&self) -> String {
        "counting-peak".into()
    }
}

#[test]
fn set_presets_always_sends_a_clear_and_every_held_preset() {
    // An explicit presets action reaches the instrument, like set_high_voltage
    // and set_coarse_gain do: equal to the mirror or not, it sends the clear
    // and each held preset. (The caller that must not disturb a count decides
    // whether to call; see the GUI's apply_presets.)
    let (mut remote, log) = connect_recording();
    let held = vec![
        "SHOW_STATUS".to_string(),
        "SET_PRESET_CLEAR".to_string(),
        "SET_PRESET_LIVE 300".to_string(),
    ];
    let same = *remote.presets();
    remote.set_presets(same).expect("same presets");
    assert_eq!(*log.lock().expect("log"), held);

    // Only the host-side presets differ: still sent, and kept in the mirror.
    log.lock().expect("log").clear();
    let mut presets = *remote.presets();
    presets.mda = Some(an_mda_preset());
    remote.set_presets(presets).expect("host-side edit");
    assert_eq!(*log.lock().expect("log"), held);
    assert_eq!(remote.presets().mda, Some(an_mda_preset()));
    assert_eq!(remote.presets().live_time, Some(300.0));

    // A real change to a held preset sends clear and then each preset.
    log.lock().expect("log").clear();
    let mut presets = *remote.presets();
    presets.live_time = Some(60.0);
    remote.set_presets(presets).expect("live preset");
    assert_eq!(
        *log.lock().expect("log"),
        vec![
            "SHOW_STATUS".to_string(),
            "SET_PRESET_CLEAR".to_string(),
            "SET_PRESET_LIVE 60".to_string()
        ]
    );
    assert_eq!(remote.presets().mda, Some(an_mda_preset()));
}

/// An instrument from before the bridge answered SHOW_PRESETS: the mirror
/// starts empty although the instrument may hold presets (the bench 926).
struct SilentAboutPresets {
    log: Arc<Mutex<Vec<String>>>,
}

impl mantaray_device::Transport for SilentAboutPresets {
    fn exchange(&mut self, command: &str) -> Result<String, mantaray_device::DeviceError> {
        self.log.lock().expect("log").push(command.to_string());
        Ok(match command {
            "SHOW_CONFIGURATION" => "MODEL=M SERIAL=S FIRMWARE=F CHANNELS=8".into(),
            "SHOW_STATUS" => "RT=0.00 LT=0.00 DT=0.0% ICR=0 ACTIVE=0 TOTAL=0".into(),
            "SHOW_DATA" => "DATA 8 0 0 0 0 0 0 0 0".into(),
            "SHOW_PRESETS" => "ERR unknown command".into(),
            _ => "OK".into(),
        })
    }

    fn peer(&self) -> String {
        "silent".into()
    }
}

#[test]
fn clear_presets_with_an_empty_mirror_still_reaches_the_instrument() {
    // The mirror says "no presets" because the instrument never said; the
    // instrument may well hold one. An explicit clear must go out.
    let log = Arc::new(Mutex::new(Vec::new()));
    let mut remote = RemoteMcb::connect(
        Box::new(SilentAboutPresets { log: log.clone() }),
        1,
        "BENCH",
    )
    .expect("connect");
    assert!(remote.presets().is_empty());
    log.lock().expect("log").clear();
    remote
        .set_presets(mantaray_device::Presets::default())
        .expect("clear");
    assert_eq!(
        *log.lock().expect("log"),
        vec!["SHOW_STATUS".to_string(), "SET_PRESET_CLEAR".to_string()]
    );
}

#[test]
fn a_refused_clear_leaves_the_host_side_presets_where_they_were() {
    // "Clear presets" asked, the instrument said no: it still holds its live
    // preset, so the mirror must still hold the MDA/uncertainty presets that
    // advance() is enforcing, not an emptied set that stops enforcing them.
    let log = Arc::new(Mutex::new(Vec::new()));
    let mut remote = RemoteMcb::connect(
        Box::new(CountingPeak {
            refuse: vec!["SET_PRESET_CLEAR"],
            log: log.clone(),
            counting: true,
        }),
        1,
        "BENCH",
    )
    .expect("connect");
    // The line reports ACTIVE=1; presets are set with the detector stopped.
    remote.stop().expect("stop");
    let uncertainty = mantaray_device::UncertaintyPreset {
        limit_percent: 25.0,
        low_channel: 2,
        high_channel: 12,
    };
    remote
        .apply_presets(mantaray_device::Presets {
            uncertainty: Some(uncertainty),
            mda: Some(an_mda_preset()),
            ..*remote.presets()
        })
        .expect("host-side presets");

    let error = remote
        .set_presets(mantaray_device::Presets::default())
        .expect_err("the clear is refused");
    assert!(error.to_string().contains("not allowed now"), "{error}");
    assert_eq!(remote.presets().uncertainty, Some(uncertainty));
    assert_eq!(remote.presets().mda, Some(an_mda_preset()));
    assert_eq!(remote.presets().live_time, Some(300.0));

    remote.start().expect("start");
    let stopped = advance(&mut remote, 2.0).expect("advance");
    assert_eq!(
        stopped,
        Some(mantaray_device::PresetKind::Uncertainty),
        "the preset the instrument never lost is still honoured"
    );
}

#[test]
fn a_refused_preset_names_every_step_that_was_not_sent() {
    let mut remote = RemoteMcb::connect(
        Box::new(Refusing {
            refuse: vec!["SET_PRESET_REAL"],
        }),
        1,
        "BENCH",
    )
    .expect("connect");
    let presets = mantaray_device::Presets {
        real_time: Some(100.0),
        live_time: Some(60.0),
        roi_peak: Some(500),
        roi_integral: Some(9000),
        ..Default::default()
    };
    let error = remote.set_presets(presets).expect_err("refused");
    let text = error.to_string();
    assert!(text.contains("presets cleared"), "{text}");
    assert!(text.contains("real-time preset not set"), "{text}");
    assert!(
        text.contains("not sent: live-time preset, ROI peak preset, ROI integral preset"),
        "{text}"
    );
    // The clear went out and the mirror agrees.
    assert_eq!(remote.presets().live_time, None);
}

#[test]
fn a_failed_bias_after_a_program_only_edit_says_the_edit_was_kept() {
    let mut remote = RemoteMcb::connect(
        Box::new(Refusing {
            refuse: vec!["SET_HV"],
        }),
        1,
        "BENCH",
    )
    .expect("connect");
    let before = remote.properties().high_voltage.target_volts;
    let mut properties = remote.properties().clone();
    properties.adc.lower_discriminator = 10;
    properties.high_voltage.target_volts = 1200.0;
    properties.amplifier.coarse_gain *= 2.0;
    let error = remote.set_properties(properties).expect_err("bias refused");
    let text = error.to_string();
    assert!(
        text.contains("ADC settings kept in this program only"),
        "{text}"
    );
    assert!(text.contains("bias not set"), "{text}");
    assert!(text.contains("not sent: coarse gain"), "{text}");
    assert_eq!(remote.properties().adc.lower_discriminator, 10);
    assert_eq!(remote.properties().high_voltage.target_volts, before);
}

use mantaray_device::Presets;

/// What MAESTRO says when a preset change is attempted during acquisition.
const NO_PRESETS_WHILE_COUNTING: &str = "Presets can't be changed during acquisition.";

/// A line whose instrument is counting (SHOW_STATUS says ACTIVE=1) and holds a
/// 300 s live preset, and which logs every command.
struct CountingLine {
    log: Arc<Mutex<Vec<String>>>,
    /// Starts counting; STOP and START move it.
    counting: bool,
}

impl mantaray_device::Transport for CountingLine {
    fn exchange(&mut self, command: &str) -> Result<String, mantaray_device::DeviceError> {
        self.log.lock().expect("log").push(command.to_string());
        match command {
            "STOP" => self.counting = false,
            "START" => self.counting = true,
            _ => {}
        }
        Ok(match command {
            "SHOW_CONFIGURATION" => "MODEL=DSPEC-50 SERIAL=1 FIRMWARE=1 CHANNELS=8".into(),
            "SHOW_STATUS" => format!(
                "RT=5.00 LT=5.00 DT=0.0% ICR=0 ACTIVE={} TOTAL=0",
                u8::from(self.counting)
            ),
            "SHOW_DATA" => "DATA 8 0 0 0 0 0 0 0 0".into(),
            "SHOW_PRESETS" => "PRESETS REAL=0.00 LIVE=300.00 PEAK=0 INTEG=0".into(),
            _ => "OK".into(),
        })
    }

    fn peer(&self) -> String {
        "counting".into()
    }
}

fn connect_counting() -> (RemoteMcb, Arc<Mutex<Vec<String>>>) {
    let log = Arc::new(Mutex::new(Vec::new()));
    let remote = RemoteMcb::connect(
        Box::new(CountingLine {
            log: log.clone(),
            counting: true,
        }),
        1,
        "BENCH",
    )
    .expect("connect");
    assert!(remote.is_active(), "the instrument says it is counting");
    log.lock().expect("log").clear();
    (remote, log)
}

#[test]
fn every_preset_road_is_refused_while_the_instrument_counts() {
    let (mut remote, log) = connect_counting();
    let before = *remote.presets();

    // A held preset edited.
    let live = Presets {
        live_time: Some(60.0),
        ..before
    };
    let error = remote.apply_presets(live).expect_err("live edit");
    assert_eq!(error.to_string(), NO_PRESETS_WHILE_COUNTING);

    // A host-side edit alone is refused too: the whole preset change is.
    let mda = Presets {
        mda: Some(an_mda_preset()),
        ..before
    };
    let error = remote.apply_presets(mda).expect_err("mda edit");
    assert_eq!(error.to_string(), NO_PRESETS_WHILE_COUNTING);

    // Apply with nothing changed, which would re-arm.
    let error = remote.apply_presets(before).expect_err("re-apply");
    assert_eq!(error.to_string(), NO_PRESETS_WHILE_COUNTING);

    // Clear presets, and the explicit primitive.
    let error = remote
        .set_presets(Presets::default())
        .expect_err("clear presets");
    assert_eq!(error.to_string(), NO_PRESETS_WHILE_COUNTING);
    let error = remote.set_presets(live).expect_err("set presets");
    assert_eq!(error.to_string(), NO_PRESETS_WHILE_COUNTING);

    // Five refused changes, one status read each, and nothing else.
    assert_eq!(
        *log.lock().expect("log"),
        vec!["SHOW_STATUS"; 5],
        "no command but the status read"
    );
    assert_eq!(*remote.presets(), before, "the mirror is untouched");
}

#[test]
fn a_setting_that_is_not_a_preset_still_changes_while_counting() {
    let (mut remote, log) = connect_counting();
    let mut properties = remote.properties().clone();
    properties.adc.lower_discriminator = 10;
    remote.set_properties(properties).expect("not a preset");
    assert_eq!(remote.properties().adc.lower_discriminator, 10);
    assert_eq!(*log.lock().expect("log"), Vec::<String>::new());
}

#[test]
fn presets_change_again_once_the_instrument_stops_counting() {
    let (mut remote, log) = connect_counting();
    remote.stop().expect("stop");
    log.lock().expect("log").clear();
    let edited = Presets {
        live_time: Some(60.0),
        ..*remote.presets()
    };
    remote.apply_presets(edited).expect("idle again");
    assert_eq!(
        *log.lock().expect("log"),
        vec![
            "SHOW_STATUS".to_string(),
            "SET_PRESET_CLEAR".to_string(),
            "SET_PRESET_LIVE 60".to_string()
        ]
    );
}

/// An instrument whose counting state the test flips from outside, as another
/// client (or the instrument itself, on reaching a preset) would. It logs every
/// command and can be made to fail its status reads.
struct Flippable {
    log: Arc<Mutex<Vec<String>>>,
    counting: Arc<std::sync::atomic::AtomicBool>,
    status_fails: Arc<std::sync::atomic::AtomicBool>,
}

impl mantaray_device::Transport for Flippable {
    fn exchange(&mut self, command: &str) -> Result<String, mantaray_device::DeviceError> {
        use std::sync::atomic::Ordering::SeqCst;
        self.log.lock().expect("log").push(command.to_string());
        Ok(match command {
            "SHOW_CONFIGURATION" => "MODEL=DSPEC-50 SERIAL=1 FIRMWARE=1 CHANNELS=8".into(),
            "SHOW_STATUS" if self.status_fails.load(SeqCst) => {
                return Err(mantaray_device::DeviceError::Communication {
                    detail: "the line dropped".into(),
                });
            }
            "SHOW_STATUS" => format!(
                "RT=5.00 LT=5.00 DT=0.0% ICR=0 ACTIVE={} TOTAL=0",
                u8::from(self.counting.load(SeqCst))
            ),
            "SHOW_DATA" => "DATA 8 0 0 0 0 0 0 0 0".into(),
            "SHOW_PRESETS" => "PRESETS REAL=0.00 LIVE=300.00 PEAK=0 INTEG=0".into(),
            _ => "OK".into(),
        })
    }

    fn peer(&self) -> String {
        "flippable".into()
    }
}

type Flags = (
    Arc<std::sync::atomic::AtomicBool>,
    Arc<std::sync::atomic::AtomicBool>,
);

fn connect_flippable(counting: bool) -> (RemoteMcb, Arc<Mutex<Vec<String>>>, Flags) {
    let log = Arc::new(Mutex::new(Vec::new()));
    let counting = Arc::new(std::sync::atomic::AtomicBool::new(counting));
    let status_fails = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let remote = RemoteMcb::connect(
        Box::new(Flippable {
            log: log.clone(),
            counting: counting.clone(),
            status_fails: status_fails.clone(),
        }),
        1,
        "BENCH",
    )
    .expect("connect");
    log.lock().expect("log").clear();
    (remote, log, (counting, status_fails))
}

#[test]
fn a_count_another_client_started_refuses_the_preset_change_and_sends_nothing() {
    use std::sync::atomic::Ordering::SeqCst;
    let (mut remote, log, (counting, _)) = connect_flippable(false);
    // Our cache says idle; someone else starts the instrument.
    counting.store(true, SeqCst);
    assert!(!remote.is_active(), "the cache has not seen it yet");
    let edited = Presets {
        live_time: Some(60.0),
        ..*remote.presets()
    };
    let error = remote.apply_presets(edited).expect_err("counting");
    assert_eq!(error.to_string(), NO_PRESETS_WHILE_COUNTING);
    let error = remote.set_presets(edited).expect_err("counting");
    assert_eq!(error.to_string(), NO_PRESETS_WHILE_COUNTING);
    // Only the status read went out, once per attempt; nothing was changed.
    assert_eq!(*log.lock().expect("log"), vec!["SHOW_STATUS"; 2]);
    assert_eq!(remote.presets().live_time, Some(300.0));
}

#[test]
fn a_count_that_stopped_itself_on_a_preset_allows_the_change() {
    use std::sync::atomic::Ordering::SeqCst;
    let (mut remote, log, (counting, _)) = connect_flippable(true);
    // Our cache says counting; the instrument reached its preset and stopped.
    counting.store(false, SeqCst);
    assert!(remote.is_active(), "the cache has not seen it yet");
    let edited = Presets {
        live_time: Some(60.0),
        ..*remote.presets()
    };
    remote
        .apply_presets(edited)
        .expect("the instrument is idle");
    assert_eq!(
        *log.lock().expect("log"),
        vec![
            "SHOW_STATUS".to_string(),
            "SET_PRESET_CLEAR".to_string(),
            "SET_PRESET_LIVE 60".to_string()
        ],
        "one status read, then the change"
    );
}

#[test]
fn a_failed_status_read_refuses_the_preset_change_with_the_reads_error() {
    use std::sync::atomic::Ordering::SeqCst;
    let (mut remote, log, (_, status_fails)) = connect_flippable(false);
    status_fails.store(true, SeqCst);
    let edited = Presets {
        live_time: Some(60.0),
        ..*remote.presets()
    };
    for error in [
        remote.apply_presets(edited).expect_err("no status"),
        remote.set_presets(edited).expect_err("no status"),
    ] {
        assert!(error.to_string().contains("the line dropped"), "{error}");
        assert!(
            matches!(error, mantaray_device::DeviceError::CountingUnknown { .. }),
            "{error}"
        );
    }
    assert_eq!(*log.lock().expect("log"), vec!["SHOW_STATUS"; 2]);
}

#[test]
fn a_non_finite_or_negative_preset_is_refused_before_anything_is_sent() {
    let (mut remote, log, _) = connect_flippable(false);
    for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -1.0] {
        let presets = Presets {
            real_time: Some(bad),
            ..*remote.presets()
        };
        assert!(remote.apply_presets(presets).is_err(), "{bad}");
        assert!(remote.set_presets(presets).is_err(), "{bad}");
        let mut properties = remote.properties().clone();
        properties.presets.live_time = Some(bad);
        assert!(remote.set_properties(properties).is_err(), "{bad}");
        let uncertainty = Presets {
            uncertainty: Some(mantaray_device::UncertaintyPreset {
                limit_percent: bad,
                low_channel: 0,
                high_channel: 3,
            }),
            ..*remote.presets()
        };
        assert!(remote.apply_presets(uncertainty).is_err(), "{bad}");
        let mda = Presets {
            mda: Some(mantaray_device::MdaPreset {
                target: bad,
                ..an_mda_preset()
            }),
            ..*remote.presets()
        };
        assert!(remote.apply_presets(mda).is_err(), "{bad}");
    }
    assert_eq!(
        *remote.presets(),
        Presets {
            live_time: Some(300.0),
            ..Presets::default()
        }
    );
    assert!(
        log.lock()
            .expect("log")
            .iter()
            .all(|command| command == "SHOW_STATUS" || command.starts_with("SHOW")),
        "no change went out: {:?}",
        log.lock().expect("log")
    );
}

/// An instrument whose answers a test decides one command at a time, logging
/// every command it is sent.
struct Answering {
    log: Arc<Mutex<Vec<String>>>,
    answer: Box<dyn FnMut(&str) -> String + Send>,
}

impl mantaray_device::Transport for Answering {
    fn exchange(&mut self, command: &str) -> Result<String, mantaray_device::DeviceError> {
        self.log.lock().expect("log").push(command.to_string());
        Ok((self.answer)(command))
    }

    fn peer(&self) -> String {
        "answering".into()
    }
}

fn connect_answering(
    answer: impl FnMut(&str) -> String + Send + 'static,
) -> (RemoteMcb, Arc<Mutex<Vec<String>>>) {
    let log = Arc::new(Mutex::new(Vec::new()));
    let remote = RemoteMcb::connect(
        Box::new(Answering {
            log: log.clone(),
            answer: Box::new(answer),
        }),
        1,
        "BENCH",
    )
    .expect("connect");
    log.lock().expect("log").clear();
    (remote, log)
}

/// A locked instrument is refused before STOP leaves, like the simulator.
#[test]
fn stop_on_a_locked_remote_sends_nothing() {
    let (mut remote, log) = connect_answering(|command| match command {
        "SHOW_CONFIGURATION" => "MODEL=M SERIAL=S FIRMWARE=F CHANNELS=4".into(),
        "SHOW_STATUS" => "RT=0.00 LT=0.00 DT=0.0% ICR=0 ACTIVE=1 TOTAL=0".into(),
        "SHOW_DATA" => "DATA 4 0 0 0 0".into(),
        "SHOW_PRESETS" => "PRESETS REAL=0 LIVE=0 PEAK=0 INTEG=0".into(),
        _ => "OK".into(),
    });
    remote.lock("pw", "someone").expect("lock");
    let error = remote.stop().expect_err("locked");
    assert!(matches!(error, mantaray_device::DeviceError::Locked { .. }));
    assert!(log.lock().expect("log").is_empty(), "{:?}", log.lock());
}

// ------------------------------------------- HIGH review of 05d1333

/// The 926's own preset verbs, which the mantaray-mcb bridge passes through
/// verbatim, are refused as raw messages: sent, they would change a preset
/// with no status read (so during a count) and the mirror would never learn
/// the new value. The refusal names the spelling that does take the rules.
#[test]
fn a_native_preset_verb_is_refused_and_points_to_set_preset() {
    let transport = MockTransport::scripted(&[
        (
            "SHOW_CONFIGURATION",
            "MODEL=DSPEC SERIAL=1 FIRMWARE=1 CHANNELS=2",
        ),
        ("SHOW_STATUS", "RT=5 LT=5 DT=0% ICR=0 ACTIVE=1 TOTAL=0"),
        ("SHOW_DATA", "DATA 2 0 0"),
        ("SHOW_PRESETS", "PRESETS REAL=0 LIVE=300 PEAK=0 INTEG=0"),
        // The typed spelling: one gate read, refused.
        ("SHOW_STATUS", "RT=5 LT=5 DT=0% ICR=0 ACTIVE=1 TOTAL=0"),
        // Nothing after this: any native verb that reached the wire, or a
        // status read made for it, would be answered with a script error.
    ]);
    let mut mcb = RemoteMcb::connect(Box::new(transport), 1, "t").expect("connect");
    let typed = mcb.send_message("SET_PRESET_LIVE 60");
    assert!(
        typed
            .as_ref()
            .is_err_and(|error| error.to_string().contains("during acquisition")),
        "{typed:?}"
    );
    for native in [
        "SET_LIVE_PRESET 3000",
        "set_true_preset 3000",
        "SET_PEAK_PRESET 100",
        "SET_INTEGRAL_PRESET 5000",
        "SET_UNCERTAINTY_PRESET 1 2 3",
        "CLEAR_PRESETS",
        "ENABLE_OVERFLOW_PRESET",
        "DISABLE_OVERFLOW_PRESET",
        "CLEAR_ALL",
    ] {
        let reply = mcb.send_message(native);
        assert!(
            reply
                .as_ref()
                .is_err_and(|error| error.to_string().contains("SET_PRESET_")),
            "{native} was not refused with a pointer to SET_PRESET_*: {reply:?}"
        );
    }
    assert_eq!(
        mcb.presets().live_time,
        Some(300.0),
        "the mirror still says 300 s"
    );
    // A preset read is still a read.
    let transport = MockTransport::scripted(&[
        (
            "SHOW_CONFIGURATION",
            "MODEL=DSPEC SERIAL=1 FIRMWARE=1 CHANNELS=2",
        ),
        ("SHOW_STATUS", "RT=5 LT=5 DT=0% ICR=0 ACTIVE=1 TOTAL=0"),
        ("SHOW_DATA", "DATA 2 0 0"),
        ("SHOW_PRESETS", "PRESETS REAL=0 LIVE=300 PEAK=0 INTEG=0"),
        ("SHOW_LIVE_PRESET", "$G0000015000081"),
    ]);
    let mut mcb = RemoteMcb::connect(Box::new(transport), 1, "t").expect("connect");
    assert_eq!(
        mcb.send_message("SHOW_LIVE_PRESET").expect("a read passes"),
        "$G0000015000081"
    );
}

/// Never started takes NEVER_STARTED_READINGS believed readings: readings
/// without the flag do not count towards it.
#[test]
fn never_started_counts_only_believed_idle_readings() {
    use mantaray_device::{CountState, CountWatch, NEVER_STARTED_READINGS};
    let idle = "RT=0 LT=0 DT=0% ICR=0 ACTIVE=0 TOTAL=0";
    let no_flag = "RT=0 LT=0 DT=0% ICR=0 TOTAL=0";
    let mut script = vec![
        (
            "SHOW_CONFIGURATION",
            "MODEL=DSPEC SERIAL=1 FIRMWARE=1 CHANNELS=2",
        ),
        ("SHOW_STATUS", idle),
        ("SHOW_DATA", "DATA 2 0 0"),
        ("SHOW_PRESETS", "PRESETS REAL=0 LIVE=10 PEAK=0 INTEG=0"),
        // start(): the already-reached read, then START.
        ("SHOW_STATUS", idle),
        ("START", "OK"),
    ];
    // Three status lines without the flag, then believed idle lines.
    for _ in 0..3 {
        script.extend([("SHOW_STATUS", no_flag), ("SHOW_DATA", "DATA 2 0 0")]);
    }
    for _ in 0..NEVER_STARTED_READINGS {
        script.extend([("SHOW_STATUS", idle), ("SHOW_DATA", "DATA 2 0 0")]);
    }
    let transport = MockTransport::scripted(&script);
    let mut mcb = RemoteMcb::connect(Box::new(transport), 1, "t").expect("connect");
    mcb.set_confirm_step(Duration::ZERO);
    mcb.start().expect("START accepted");
    let mut watch = CountWatch::started(&mcb);
    let mut states = Vec::new();
    for _ in 0..3 + NEVER_STARTED_READINGS {
        mcb.poll(0.5).expect("poll");
        let state = watch.check(&mcb);
        // A second look at the same reading is not a second reading.
        assert_eq!(watch.check(&mcb), state);
        states.push(state);
    }
    let last = states.pop();
    assert!(
        states.iter().all(|state| *state == CountState::Counting),
        "called before {NEVER_STARTED_READINGS} believed readings: {states:?}"
    );
    assert_eq!(last, Some(CountState::NeverStarted));
}

// ------------------------------------------- HIGH review of 4ae6457

const IDLE0: &str = "RT=0.00 LT=0.00 DT=0% ICR=0 ACTIVE=0 TOTAL=0";

fn connect_live_10(script: &[(&str, &str)]) -> RemoteMcb {
    let mut all = vec![
        (
            "SHOW_CONFIGURATION",
            "MODEL=DSPEC SERIAL=1 FIRMWARE=1 CHANNELS=2",
        ),
        ("SHOW_STATUS", IDLE0),
        ("SHOW_DATA", "DATA 2 0 0"),
        ("SHOW_PRESETS", "PRESETS REAL=0 LIVE=10 PEAK=0 INTEG=0"),
    ];
    all.extend_from_slice(script);
    let mut mcb =
        RemoteMcb::connect(Box::new(MockTransport::scripted(&all)), 1, "t").expect("connect");
    mcb.set_confirm_step(Duration::ZERO);
    mcb
}

/// A first reading after START that says idle while the clocks already run
/// (the flag trailing the clocks, or the bridge's flag-first status
/// straddling the moment counting begins) is not the end of the count.
#[test]
fn a_flag_trailing_running_clocks_is_not_a_count_over() {
    use mantaray_device::{CountState, CountWatch};
    let mut mcb = connect_live_10(&[
        ("SHOW_STATUS", IDLE0),
        ("START", "OK"),
        (
            "SHOW_STATUS",
            "RT=0.40 LT=0.40 DT=0% ICR=0 ACTIVE=0 TOTAL=3",
        ),
        ("SHOW_DATA", "DATA 2 1 2"),
        // A poll later, the clocks stood still: it ran 0.4 s and is over.
        (
            "SHOW_STATUS",
            "RT=0.40 LT=0.40 DT=0% ICR=0 ACTIVE=0 TOTAL=3",
        ),
        ("SHOW_DATA", "DATA 2 1 2"),
    ]);
    mcb.start().expect("START accepted");
    let mut watch = CountWatch::started(&mcb);
    mcb.poll(0.0).expect("poll");
    assert_eq!(
        watch.check(&mcb),
        CountState::Counting,
        "a reading 0.4 s into a 10 s live count ended the WAIT"
    );
    assert_eq!(watch.check(&mcb), CountState::Counting, "a second look");
    // A poll later (wall time apart), the clocks stood still: over.
    std::thread::sleep(Duration::from_millis(300));
    mcb.poll(0.5).expect("poll");
    assert_eq!(
        watch.check(&mcb),
        CountState::Over,
        "clocks that stood still"
    );
}

/// Medium review of 12b4d2a: a fetch forced out right behind another (a
/// gate read sets the next poll due at once) that reads the same clocks
/// decides nothing: neither the WAIT nor the preset hold ends on it.
#[test]
fn clocks_read_twice_in_a_moment_did_not_stand_still() {
    use mantaray_device::{CountState, CountWatch};
    let running = "RT=0.40 LT=0.40 DT=0% ICR=0 ACTIVE=0 TOTAL=3";
    let mut mcb = connect_live_10(&[
        ("SHOW_STATUS", IDLE0),
        ("START", "OK"),
        ("SHOW_STATUS", running),
        ("SHOW_DATA", "DATA 2 1 2"),
        ("SHOW_STATUS", running),
        ("SHOW_DATA", "DATA 2 1 2"),
        // The gate read for a preset write.
        ("SHOW_STATUS", running),
    ]);
    mcb.start().expect("START accepted");
    let mut watch = CountWatch::started(&mcb);
    mcb.poll(0.5).expect("poll");
    assert_eq!(watch.check(&mcb), CountState::Counting);
    mcb.poll(0.5).expect("a fetch a moment later");
    assert_eq!(
        watch.check(&mcb),
        CountState::Counting,
        "two fetches a moment apart ended the WAIT"
    );
    let result = mcb.set_presets(Presets {
        live_time: Some(20.0),
        ..Presets::default()
    });
    assert!(
        matches!(
            result,
            Err(mantaray_device::DeviceError::PresetsWhileCounting)
        ),
        "two fetches a moment apart released the hold: {result:?}"
    );
}

/// Medium review of 12b4d2a: clocks that go back under a watch were cleared
/// (by whoever): the readings after decide afresh, never "stood still".
#[test]
fn clocks_cleared_under_a_watch_are_not_a_count_over() {
    use mantaray_device::{CountState, CountWatch};
    let mut mcb = connect_live_10(&[
        ("SHOW_STATUS", IDLE0),
        ("START", "OK"),
        (
            "SHOW_STATUS",
            "RT=0.40 LT=0.40 DT=0% ICR=0 ACTIVE=0 TOTAL=3",
        ),
        ("SHOW_DATA", "DATA 2 1 2"),
        // Someone cleared it; it counts on, flag still trailing.
        (
            "SHOW_STATUS",
            "RT=0.20 LT=0.20 DT=0% ICR=0 ACTIVE=0 TOTAL=1",
        ),
        ("SHOW_DATA", "DATA 2 0 1"),
    ]);
    mcb.start().expect("START accepted");
    let mut watch = CountWatch::started(&mcb);
    mcb.poll(0.5).expect("poll");
    assert_eq!(watch.check(&mcb), CountState::Counting);
    std::thread::sleep(Duration::from_millis(300));
    mcb.poll(0.5).expect("poll");
    assert_eq!(
        watch.check(&mcb),
        CountState::Counting,
        "a clock reset was read as clocks that stood still"
    );
}

/// The same reading keeps the preset hold: a preset write (with the CLEAR it
/// carries) is refused while the clocks run on behind an idle flag.
#[test]
fn a_flag_trailing_running_clocks_keeps_the_preset_hold() {
    let mut mcb = connect_live_10(&[
        ("SHOW_STATUS", IDLE0),
        ("START", "OK"),
        (
            "SHOW_STATUS",
            "RT=0.40 LT=0.40 DT=0% ICR=0 ACTIVE=0 TOTAL=3",
        ),
        ("SHOW_DATA", "DATA 2 1 2"),
        (
            "SHOW_STATUS",
            "RT=0.90 LT=0.90 DT=0% ICR=0 ACTIVE=0 TOTAL=7",
        ),
    ]);
    let sent = chrono::Local::now().naive_local();
    mcb.start().expect("START accepted");
    mcb.poll(0.0).expect("poll");
    let result = mcb.set_presets(Presets {
        live_time: Some(20.0),
        ..Presets::default()
    });
    assert!(
        matches!(
            result,
            Err(mantaray_device::DeviceError::PresetsWhileCounting)
        ),
        "a preset write reached a count whose clocks were running: {result:?}"
    );
    // Clocks that ran on twice date the count from the second reading,
    // 0.9 s before it: before the START here, so no earlier than the START.
    let dated = mcb.spectrum().start_time.expect("a date");
    assert!(
        dated >= sent && dated <= chrono::Local::now().naive_local(),
        "dated {dated}, START sent {sent}"
    );
}

/// Medium review of e8683de: a job's CLEAR after a count that ran out with
/// no look at it (START / WAIT 60 / CLEAR / WAIT) leaves a WAIT that ends
/// on the count being over, never on "did not count".
#[test]
fn a_clear_after_an_unwatched_count_is_over_not_never_started() {
    use mantaray_device::{CountState, CountWatch, NEVER_STARTED_READINGS};
    let ran = "RT=1.00 LT=1.00 DT=0% ICR=0 ACTIVE=0 TOTAL=5";
    let mut script = vec![
        ("SHOW_STATUS", IDLE0),
        ("START", "OK"),
        ("SHOW_STATUS", ran),
        ("SHOW_DATA", "DATA 2 2 3"),
        ("CLEAR", "OK"),
    ];
    for _ in 0..NEVER_STARTED_READINGS + 2 {
        script.extend([("SHOW_STATUS", IDLE0), ("SHOW_DATA", "DATA 2 0 0")]);
    }
    let mut mcb = connect_live_10(&script);
    mcb.start().expect("START accepted");
    let mut watch = CountWatch::started(&mcb);
    // WAIT 60: polls, no look.
    mcb.poll(0.5).expect("poll");
    watch.clearing(&mcb);
    mcb.clear().expect("CLEAR");
    let mut state = CountState::Counting;
    for _ in 0..NEVER_STARTED_READINGS + 2 {
        std::thread::sleep(Duration::from_millis(30));
        mcb.poll(0.5).expect("poll");
        state = watch.check(&mcb);
        if state != CountState::Counting {
            break;
        }
    }
    assert_eq!(state, CountState::Over, "the count ran; it is over");
}

/// Medium review of e8683de: a gate read's clocks are judged with the gate
/// read's time, so a fetch the gate forced out right behind it on the same
/// clocks is not "stood still".
#[test]
fn a_gate_read_and_the_fetch_behind_it_did_not_stand_still() {
    use mantaray_device::{CountState, CountWatch};
    let mut mcb = connect_live_10(&[
        ("SHOW_STATUS", IDLE0),
        ("START", "OK"),
        (
            "SHOW_STATUS",
            "RT=1.00 LT=1.00 DT=0% ICR=0 ACTIVE=0 TOTAL=5",
        ),
        ("SHOW_DATA", "DATA 2 2 3"),
        // The gate read for a refused preset write.
        (
            "SHOW_STATUS",
            "RT=1.40 LT=1.40 DT=0% ICR=0 ACTIVE=0 TOTAL=7",
        ),
        // The fetch it forces out, a moment later.
        (
            "SHOW_STATUS",
            "RT=1.40 LT=1.40 DT=0% ICR=0 ACTIVE=0 TOTAL=7",
        ),
        ("SHOW_DATA", "DATA 2 3 4"),
    ]);
    mcb.start().expect("START accepted");
    let mut watch = CountWatch::started(&mcb);
    mcb.poll(0.5).expect("poll");
    std::thread::sleep(Duration::from_millis(300));
    let refused = mcb.set_presets(Presets {
        live_time: Some(20.0),
        ..Presets::default()
    });
    assert!(refused.is_err(), "{refused:?}");
    assert_eq!(watch.check(&mcb), CountState::Counting);
    mcb.poll(0.5).expect("the forced fetch");
    assert_eq!(
        watch.check(&mcb),
        CountState::Counting,
        "a gate read and the fetch right behind it ended the WAIT"
    );
}

/// Third HIGH review (of 84ab2f1), nit 1: after a CLEAR, the preset hold
/// agrees with the count watch about a count that ran unseen. Its clocks
/// stand still at zero, the WAIT says over, and a preset write right after
/// is let through (it was refused for some 20 more fetches).
#[test]
fn a_clear_after_an_unwatched_count_releases_the_hold_with_the_watch() {
    use mantaray_device::{CountState, CountWatch};
    let mut mcb = connect_live_10(&[
        ("SHOW_STATUS", IDLE0),
        ("START", "OK"),
        (
            "SHOW_STATUS",
            "RT=1.00 LT=1.00 DT=0% ICR=0 ACTIVE=0 TOTAL=5",
        ),
        ("SHOW_DATA", "DATA 2 2 3"),
        ("CLEAR", "OK"),
        ("SHOW_STATUS", IDLE0),
        ("SHOW_DATA", "DATA 2 0 0"),
        ("SHOW_STATUS", IDLE0),
        ("SHOW_DATA", "DATA 2 0 0"),
        // The preset write's gate read, then the write.
        ("SHOW_STATUS", IDLE0),
        ("SET_PRESET_CLEAR", "OK"),
        ("SET_PRESET_LIVE 5", "OK"),
    ]);
    mcb.start().expect("START accepted");
    let mut watch = CountWatch::started(&mcb);
    mcb.poll(0.5).expect("poll");
    watch.clearing(&mcb);
    mcb.clear().expect("CLEAR");
    mcb.poll(0.5).expect("poll");
    assert_eq!(watch.check(&mcb), CountState::Counting);
    std::thread::sleep(Duration::from_millis(300));
    mcb.poll(0.5).expect("poll");
    assert_eq!(
        watch.check(&mcb),
        CountState::Over,
        "the count ran; it is over"
    );
    mcb.set_presets(Presets {
        live_time: Some(5.0),
        ..Presets::default()
    })
    .expect("the hold agrees the count is over");
}
