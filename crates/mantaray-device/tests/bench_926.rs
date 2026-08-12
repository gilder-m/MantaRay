//! The bench check, against a real ORTEC 926 on the USB bus.
//!
//! Ignored by default: it needs the instrument, so CI cannot run it and a
//! working copy without hardware must not fail. Run it where the 926 is:
//!
//! ```text
//! cargo build --release -p mantaray-mcb
//! cargo test -p mantaray-device --test bench_926 -- --ignored --nocapture
//! ```
//!
//! `MANTARAY_MCB` overrides where the helper is; `MANTARAY_BENCH_SERIAL`
//! overrides which adapter is expected.

use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};

use mantaray_device::{BridgeTransport, Mcb, RemoteMcb};

/// One instrument, one holder.
///
/// The adapter is claimed exclusively, so only one of these tests can hold it
/// at a time. Cargo runs them in parallel by default, and without this the
/// others find the interface already taken and fail saying so - which looks
/// like a broken instrument rather than like tests standing on each other.
/// `bridge_hardware.rs` takes turns the same way, for the same reason.
static BENCH: Mutex<()> = Mutex::new(());

/// Waits for the bench, ignoring a previous test having panicked while holding
/// it - the lock orders access, it does not guard any state.
fn bench() -> MutexGuard<'static, ()> {
    BENCH.lock().unwrap_or_else(|held| held.into_inner())
}

/// The helper the bridge runs.
///
/// Found relative to this test binary rather than to the source tree, because
/// the two are not always in the same place: `CARGO_TARGET_DIR`, or a
/// `target-dir` in any `config.toml`, puts build output wherever the developer
/// wants it. This binary is at `<target>/debug/deps/`, so the release
/// directory is three levels up and across - true wherever `<target>` is.
fn helper() -> PathBuf {
    if let Ok(path) = std::env::var("MANTARAY_MCB") {
        return PathBuf::from(path);
    }
    let from_target = std::env::current_exe().ok().and_then(|exe| {
        Some(
            exe.parent()? // deps
                .parent()? // debug
                .parent()? // target
                .join("release"),
        )
    });
    from_target
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/release"))
        .join(mantaray_device::BRIDGE_EXECUTABLE)
}

/// Opens the instrument the way a saved entry does: the serial pins which
/// adapter the helper opens, and is also what the client checks it against.
fn open(expected: Option<&str>) -> Result<RemoteMcb, String> {
    let transport = BridgeTransport::start_pinned(&helper(), 1, expected, None)
        .map_err(|error| error.to_string())?;
    RemoteMcb::connect_expecting(Box::new(transport), 1, "Bench 926", expected)
        .map_err(|error| error.to_string())
}

/// Opens whatever adapter is first and checks its identity only at the client.
///
/// This is the route the Windows library takes, where selection is by
/// detector number and the helper cannot pin an adapter - and it is the only
/// way to reach `connect_expecting`'s own refusal from here, because a pinned
/// helper refuses a serial it does not have before a reply is ever exchanged.
fn open_unpinned_expecting(expected: &str) -> Result<RemoteMcb, String> {
    let transport =
        BridgeTransport::start_pinned(&helper(), 1, None, None).map_err(|e| e.to_string())?;
    RemoteMcb::connect_expecting(Box::new(transport), 1, "Bench 926", Some(expected))
        .map_err(|error| error.to_string())
}

#[test]
#[ignore = "needs the ORTEC 926 on the bus; run with --ignored"]
fn the_instrument_is_learned_then_matched_and_a_stranger_is_refused() {
    let _bench = bench();
    // First open of a new entry: nothing is expected, so the serial is
    // learned from what the instrument calls itself.
    let learned = {
        let instrument = open(None).expect("the 926 opens");
        let identity = instrument.identity();
        println!(
            "learned: model {:?} serial {:?} firmware {:?}, {} channels",
            identity.model, identity.serial, identity.firmware, identity.channels
        );
        assert!(
            !identity.serial.trim().is_empty(),
            "the instrument must report a serial for any of this to mean anything"
        );
        identity.serial.clone()
    };
    if let Ok(expected) = std::env::var("MANTARAY_BENCH_SERIAL") {
        assert_eq!(learned, expected, "not the adapter this bench expects");
    }

    // Second open of the same entry: the remembered serial must match, and
    // the detector must open exactly as before.
    let again = open(Some(&learned)).expect("the same instrument opens again");
    assert_eq!(again.identity().serial, learned);
    println!("matched: {learned} opened again under its own entry");
    drop(again);

    // A different instrument under this entry must not open. Two guards catch
    // that, and both are checked because they fail in different places.
    let stranger = format!("{learned}-not");

    // The helper's own: it will not open an adapter whose serial it cannot
    // find, so nothing is exchanged at all.
    let pinned = match open(Some(&stranger)) {
        Ok(_) => panic!("a stranger must not open under this entry"),
        Err(error) => error,
    };
    println!("pinned helper refused: {pinned}");

    // The client's own, on the route where the helper cannot pin anything:
    // the configuration reply is compared, and the refusal names both.
    let checked = match open_unpinned_expecting(&stranger) {
        Ok(_) => panic!("the identity check must refuse a serial that does not match"),
        Err(error) => error,
    };
    println!("client refused: {checked}");
    assert!(
        checked.contains(&stranger) && checked.contains(&learned),
        "the refusal should name what answered and what was expected: {checked}"
    );
}

#[test]
#[ignore = "needs the ORTEC 926 on the bus; run with --ignored"]
fn the_presets_the_instrument_holds_are_read_on_connecting() {
    let _bench = bench();
    // The bug this covers, found on the bench: preset registers outlive the
    // session that wrote them, and mantaray only ever wrote them - so it
    // opened showing none while the instrument held one.
    let instrument = open(None).expect("the 926 opens");
    let presets = instrument.presets();
    let status = instrument.status();
    println!(
        "presets: real {:?} live {:?} peak {:?} integral {:?}",
        presets.real_time, presets.live_time, presets.roi_peak, presets.roi_integral
    );
    println!(
        "status: RT={:.2} LT={:.2} active={}",
        status.real_time, status.live_time, status.active
    );
    // Whatever it holds, what it reports must be self-consistent: a preset
    // read back as Some is a positive number, never a zero standing in for
    // "none set" (which would stop an acquisition the instant it began).
    for (name, seconds) in [("real", presets.real_time), ("live", presets.live_time)] {
        if let Some(seconds) = seconds {
            assert!(seconds > 0.0, "{name} preset read back as {seconds}");
        }
    }
}

#[test]
#[ignore = "needs the ORTEC 926 on the bus; run with --ignored"]
fn a_preset_the_instrument_has_already_reached_refuses_the_next_start() {
    let _bench = bench();
    // The whole point of reading the presets back, proven end to end on the
    // instrument: count out a short preset, then try to start again. The 926
    // answers START, leaves the clocks where they are and says nothing, so the
    // refusal has to come from here - and it has to name the preset.
    let mut instrument = open(None).expect("the 926 opens");
    let held = *instrument.presets();
    println!("holding: {held:?}");

    // A preset short enough to watch reach, from a cleared spectrum.
    instrument.clear().expect("clear");
    let brief = mantaray_device::Presets {
        live_time: Some(1.0),
        ..Default::default()
    };
    instrument.set_presets(brief).expect("a one-second preset");
    instrument.start().expect("start");

    // The instrument stops itself; poll until it says so.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while instrument.is_active() {
        assert!(
            std::time::Instant::now() < deadline,
            "the instrument never stopped on a one-second live preset"
        );
        std::thread::sleep(std::time::Duration::from_millis(250));
        instrument.poll(1.0).expect("poll");
    }
    let status = instrument.status();
    println!(
        "stopped itself at RT={:.2} LT={:.2}",
        status.real_time, status.live_time
    );
    assert!(
        status.live_time >= 1.0,
        "the live preset should have been reached, got LT={}",
        status.live_time
    );

    // Now the state that used to swallow a START.
    let error = match instrument.start() {
        Ok(()) => panic!("starting against a reached preset must not be accepted"),
        Err(error) => error.to_string(),
    };
    println!("refused: {error}");
    assert!(
        error.contains("Live time"),
        "the refusal should name the preset: {error}"
    );

    // Put the instrument back the way it was found.
    instrument
        .set_presets(held)
        .expect("the held presets go back");
    instrument.clear().expect("clear");
}

/// A count already running when the window opens keeps its start date.
///
/// Found on the bench and reported plainly: start an acquisition, close
/// MantaRay, open it again, and the start time was gone - so a `.Spe` saved
/// afterwards carried none, and the writer filled that gap with the Unix
/// epoch, which reads back as a measurement made in 1970.
///
/// Nothing on this road reports a measurement date. `MIOGetStartTime` is in
/// ORTEC's Windows library and has no counterpart here, so the start is
/// reconstructed from the real-time clock, which advances only while the run
/// does. This is that reconstruction, against a real instrument and a real
/// clock rather than a simulated one.
#[test]
#[ignore = "needs the ORTEC 926 on the bus; run with --ignored"]
fn a_count_already_running_when_the_window_opens_keeps_its_start() {
    let _bench = bench();
    let held = {
        let mut instrument = open(None).expect("the 926 opens");
        let held = *instrument.presets();
        // No preset, or a short one would stop the run before the second
        // session ever sees it counting.
        instrument
            .set_presets(mantaray_device::Presets::default())
            .expect("presets clear");
        instrument.clear().expect("clear");
        instrument.start().expect("start");
        std::thread::sleep(std::time::Duration::from_secs(4));
        instrument.poll(4.0).expect("poll");
        assert!(
            instrument.spectrum().start_time.is_some(),
            "the session that started it must know the date"
        );
        held
        // Dropped here, which is what closing the window does.
    };

    // A second session, which never saw the run begin.
    let mut instrument = open(None).expect("the 926 opens again");
    instrument.poll(0.0).expect("poll");
    let status = instrument.status();
    assert!(
        status.active,
        "the instrument should still be counting: RT={:.2}",
        status.real_time
    );
    let recovered = instrument
        .spectrum()
        .start_time
        .expect("a run in progress must carry a start date");
    let ago = (chrono::Local::now().naive_local() - recovered).num_milliseconds() as f64 / 1000.0;
    println!(
        "recovered start {recovered}, {ago:.2} s ago, RT={:.2}",
        status.real_time
    );
    assert!(
        (ago - status.real_time).abs() < 3.0,
        "the start should sit one real-time clock back: {ago:.2} s ago against RT={:.2} s",
        status.real_time
    );

    // Put the instrument back the way it was found.
    instrument.stop().expect("stop");
    instrument.clear().expect("clear");
    instrument
        .set_presets(held)
        .expect("the held presets go back");
}
