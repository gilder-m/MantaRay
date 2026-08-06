//! The remote-instrument layer, proven without hardware.
//!
//! Two levels: the wire protocol against a scripted transport (every byte
//! checked), and a full round trip over real TCP against a served simulator -
//! the same code path a bench instrument will use, minus the bench.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use ortseam_device::{AnyMcb, Mcb, MockTransport, RemoteMcb, SimulatedMcb, TcpTransport, advance};

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
    // in one session stays in the instrument's registers, but ortseam only
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
    let presets = ortseam_device::Presets {
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
            let _ = ortseam_device::server::serve_connection(stream, behind);
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

    let presets = ortseam_device::Presets {
        uncertainty: Some(ortseam_device::UncertaintyPreset {
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
        Some(ortseam_device::PresetKind::Uncertainty),
        "the uncertainty preset should have stopped the count"
    );
    assert!(!remote.is_active(), "and the instrument should be stopped");
}
