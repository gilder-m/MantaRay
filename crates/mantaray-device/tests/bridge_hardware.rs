//! The whole chain against a real instrument: MantaRay's dialect, through the
//! bridge process, over USB, with none of ORTEC's user-mode software.
//!
//! This is the test that says a machine which never had MAESTRO installed can
//! drive its own detector. It needs hardware, so it **skips out loud** when
//! there is none - a green run that reached nothing must not be mistakable for
//! a green run that reached an instrument.
//!
//! Point it at the bridge with `MANTARAY_MCB`; it is skipped entirely without
//! that, because guessing at a path would silently test nothing.
//!
//! ```text
//! MANTARAY_MCB=target/i686-pc-windows-msvc/release/mantaray-mcb.exe \
//!   cargo test -p mantaray-device --test bridge_hardware -- --nocapture
//! ```

use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};

use mantaray_device::{BridgeTransport, Transport};

/// One instrument, one holder.
///
/// An adapter can be open in one process at a time, so tests that reach the
/// bench have to take turns. Cargo runs them in parallel by default, and
/// without this the second to start finds the detector already claimed and
/// fails for a reason that has nothing to do with what it was testing.
static BENCH: Mutex<()> = Mutex::new(());

/// Waits for the bench, ignoring a previous test having panicked while holding
/// it - the lock orders access, it does not guard any state.
fn bench() -> MutexGuard<'static, ()> {
    BENCH.lock().unwrap_or_else(|held| held.into_inner())
}

/// The bridge to drive, if one was named and exists.
fn bridge() -> Option<PathBuf> {
    let path = PathBuf::from(std::env::var("MANTARAY_MCB").ok()?);
    path.is_file().then_some(path)
}

fn skip(why: &str) -> bool {
    eprintln!("skipping: {why}");
    true
}

/// Starts the bridge, or explains why it could not.
fn connect() -> Option<BridgeTransport> {
    let path = bridge()?;
    match BridgeTransport::start(&path, 1, None) {
        Ok(transport) => Some(transport),
        Err(error) => {
            eprintln!("skipping: the bridge would not start ({error})");
            None
        }
    }
}

#[test]
fn the_whole_chain_reaches_a_real_instrument() {
    let _turn = bench();
    let Some(path) = bridge() else {
        assert!(skip("MANTARAY_MCB does not name a bridge executable"));
        return;
    };
    eprintln!("driving {}", path.display());
    let Some(mut transport) = connect() else {
        return;
    };

    // What the instrument is. A configuration line is the first thing MantaRay
    // asks for, and everything downstream is scaled by the channel count in it.
    let configuration = match transport.exchange("SHOW_CONFIGURATION") {
        Ok(reply) => reply,
        Err(error) => {
            eprintln!("skipping: no instrument answered ({error})");
            return;
        }
    };
    eprintln!("configuration: {configuration}");
    assert!(
        configuration.starts_with("MODEL="),
        "a configuration line begins with the model: {configuration:?}"
    );
    let channels: usize = configuration
        .split_whitespace()
        .find_map(|field| field.strip_prefix("CHANNELS="))
        .and_then(|value| value.parse().ok())
        .unwrap_or_else(|| panic!("no channel count in {configuration:?}"));
    assert!(
        channels.is_power_of_two() && (256..=65536).contains(&channels),
        "{channels} channels is not a conversion gain an MCB offers"
    );

    // The clocks, and the arithmetic MantaRay does on them.
    let status = transport.exchange("SHOW_STATUS").expect("a status line");
    eprintln!("status: {status}");
    let field = |name: &str| -> f64 {
        status
            .split_whitespace()
            .find_map(|f| f.strip_prefix(name))
            .and_then(|v| v.trim_end_matches('%').parse().ok())
            .unwrap_or_else(|| panic!("no {name} in {status:?}"))
    };
    let (real, live) = (field("RT="), field("LT="));
    assert!(
        real >= 0.0 && live >= 0.0,
        "a clock ran backwards: {status}"
    );
    assert!(
        live <= real + 1e-6,
        "live time {live} exceeds real time {real}"
    );

    // The spectrum, and the one check that it is the instrument's own numbers:
    // the channels must sum to what the instrument reports as its total.
    let data = transport.exchange("SHOW_DATA").expect("a data line");
    let mut fields = data.split_whitespace();
    assert_eq!(fields.next(), Some("DATA"), "a data line begins with DATA");
    let declared: usize = fields.next().expect("a count").parse().expect("a number");
    assert_eq!(declared, channels, "the data is not the whole spectrum");
    let counts: Vec<u64> = fields
        .map(|value| value.parse().expect("a count"))
        .collect();
    assert_eq!(
        counts.len(),
        channels,
        "short by {} channels",
        channels - counts.len()
    );

    let total: u64 = counts.iter().sum();
    let after = transport.exchange("SHOW_STATUS").expect("a status line");
    let reported: u64 = after
        .split_whitespace()
        .find_map(|f| f.strip_prefix("TOTAL="))
        .and_then(|v| v.parse().ok())
        .expect("a total");
    assert_eq!(
        total, reported,
        "the channels sum to {total} but the bridge reports {reported}"
    );
    eprintln!("{channels} channels, {total} counts, agreeing with the bridge's own total");
}

#[test]
fn starting_and_stopping_moves_the_instrument() {
    let _turn = bench();
    let Some(_) = bridge() else {
        assert!(skip("MANTARAY_MCB does not name a bridge executable"));
        return;
    };
    let Some(mut transport) = connect() else {
        return;
    };
    let Ok(before) = transport.exchange("SHOW_STATUS") else {
        eprintln!("skipping: no instrument answered");
        return;
    };
    let active = |line: &str| {
        line.split_whitespace()
            .find_map(|f| f.strip_prefix("ACTIVE="))
            .map(|v| v == "1")
            .unwrap_or(false)
    };
    let was_running = active(&before);

    transport.exchange("START").expect("START is accepted");
    let running = transport.exchange("SHOW_STATUS").expect("a status line");
    assert!(active(&running), "the instrument did not start: {running}");

    transport.exchange("STOP").expect("STOP is accepted");
    let stopped = transport.exchange("SHOW_STATUS").expect("a status line");
    assert!(!active(&stopped), "the instrument did not stop: {stopped}");

    // Put it back the way it was found, because this is somebody's detector.
    if was_running {
        let _ = transport.exchange("START");
    }
    eprintln!("START and STOP both took effect, and the instrument was left as found");
}
