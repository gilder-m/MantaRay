//! The remote-instrument layer, proven without hardware.
//!
//! Two levels: the wire protocol against a scripted transport (every byte
//! checked), and a full round trip over real TCP against a served simulator -
//! the same code path a bench instrument will use, minus the bench.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use ortseam_device::{AnyMcb, Mcb, MockTransport, RemoteMcb, SimulatedMcb, TcpTransport, advance};

fn connect_scripted(pairs: &[(&str, &str)]) -> RemoteMcb {
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
