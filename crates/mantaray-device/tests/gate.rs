//! The instrument-control rules (device-spec.md, Part 1), pinned once.
//!
//! Every command that changes a detector goes through one gate: locked, then
//! (presets only) counting, asked of the instrument right then, then the
//! values. These tests run the same cases against the simulator and a
//! network instrument on a recording, scriptable line (the contract table,
//! T29), and pin the network instrument's own wire behavior: what is sent,
//! in what order, and that "counting" is only ever a reading.
//!
//! The T-numbers are the spec's (section 2.11).

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use mantaray_device::{
    AnyMcb, DeviceError, Mcb, MdaPreset, PresetKind, Presets, RemoteMcb, SimulatedMcb, Transport,
    UncertaintyPreset, advance,
};

// ---------------------------------------------------------------------------
// The scriptable instrument.

/// What the far end is doing, and everything it has been sent.
struct Bench {
    log: Vec<String>,
    /// Whether the instrument is counting (its own state, not the mirror's).
    counting: bool,
    /// Whether START makes it count (false: the 926's already-reached preset).
    start_counts: bool,
    /// Whether STOP makes it stop.
    stop_works: bool,
    /// Status reads after a STOP that still say ACTIVE=1.
    stop_lag: usize,
    /// Status reads after a START that still say ACTIVE=0.
    start_lag: usize,
    lag_left: usize,
    lag_value: bool,
    /// Clocks the status reports.
    clock: f64,
    /// Counts in every channel of SHOW_DATA.
    counts: u64,
    /// Replies scripted for the next SHOW_STATUS reads, in order.
    status_queue: VecDeque<String>,
    /// A reply every SHOW_STATUS gets while set (after the queue).
    status_always: Option<String>,
    /// A reply every SHOW_DATA gets while set.
    data_always: Option<String>,
    /// Fixed replies by command (e.g. "STOP" -> "ERR ...").
    replies: HashMap<String, String>,
    /// The SHOW_PRESETS answer; `None` is an old bridge that refuses it.
    shown: Option<String>,
}

impl Bench {
    fn new() -> Self {
        Self {
            log: Vec::new(),
            counting: false,
            start_counts: true,
            stop_works: true,
            stop_lag: 0,
            start_lag: 0,
            lag_left: 0,
            lag_value: false,
            clock: 0.0,
            counts: 0,
            status_queue: VecDeque::new(),
            status_always: None,
            data_always: None,
            replies: HashMap::new(),
            shown: Some("PRESETS REAL=0.00 LIVE=0.00 PEAK=0 INTEG=0".into()),
        }
    }

    fn status_line(&mut self) -> String {
        if let Some(line) = self.status_queue.pop_front() {
            return line;
        }
        if let Some(line) = &self.status_always {
            return line.clone();
        }
        let active = if self.lag_left > 0 {
            self.lag_left -= 1;
            self.lag_value
        } else {
            self.counting
        };
        format!(
            "RT={c:.2} LT={c:.2} DT=0.0% ICR=0 ACTIVE={} TOTAL=0",
            u8::from(active),
            c = self.clock
        )
    }
}

#[derive(Clone)]
struct Line(Arc<Mutex<Bench>>);

impl Transport for Line {
    fn exchange(&mut self, command: &str) -> Result<String, DeviceError> {
        let mut bench = self.0.lock().expect("bench");
        bench.log.push(command.to_string());
        if let Some(reply) = bench.replies.get(command).cloned() {
            return Ok(reply);
        }
        Ok(match command {
            "SHOW_CONFIGURATION" => "MODEL=DSPEC-50 SERIAL=1 FIRMWARE=1 CHANNELS=8".into(),
            "SHOW_STATUS" => bench.status_line(),
            "SHOW_DATA" => match &bench.data_always {
                Some(line) => line.clone(),
                None => format!("DATA 8{}", format!(" {}", bench.counts).repeat(8)),
            },
            "SHOW_PRESETS" => match &bench.shown {
                Some(text) => text.clone(),
                None => "ERR SHOW_PRESETS is not a command this bridge knows".into(),
            },
            "START" => {
                if bench.start_counts {
                    bench.counting = true;
                    if bench.start_lag > 0 {
                        bench.lag_left = bench.start_lag;
                        bench.lag_value = false;
                    }
                }
                "OK".into()
            }
            "STOP" => {
                if bench.stop_works {
                    bench.counting = false;
                    if bench.stop_lag > 0 {
                        bench.lag_left = bench.stop_lag;
                        bench.lag_value = true;
                    }
                }
                "OK".into()
            }
            "CLEAR" => {
                bench.clock = 0.0;
                bench.counts = 0;
                "OK".into()
            }
            _ => "OK".into(),
        })
    }

    fn peer(&self) -> String {
        "bench".into()
    }
}

type Shared = Arc<Mutex<Bench>>;

/// A strong peak: its uncertainty is far below a percent.
const PEAK: &str = "DATA 8 1 1 100000 1000000 100000 1 1 1";

fn make_bench(set: impl FnOnce(&mut Bench)) -> Shared {
    let mut b = Bench::new();
    set(&mut b);
    Arc::new(Mutex::new(b))
}

/// A network instrument on `bench`, log cleared after connecting.
fn remote_on(bench: &Shared) -> RemoteMcb {
    let mut remote =
        RemoteMcb::connect(Box::new(Line(bench.clone())), 1, "BENCH").expect("connect");
    // The confirm reads count, not their spacing: no test sleeps through them.
    remote.set_confirm_step(Duration::ZERO);
    bench.lock().expect("bench").log.clear();
    remote
}

fn log(bench: &Shared) -> Vec<String> {
    bench.lock().expect("bench").log.clone()
}

fn clear_log(bench: &Shared) {
    bench.lock().expect("bench").log.clear();
}

fn sets(bench: &Shared) -> Vec<String> {
    log(bench)
        .into_iter()
        .filter(|line| !line.starts_with("SHOW_"))
        .collect()
}

fn strs(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

// ---------------------------------------------------------------------------
// The contract table (T29): one rig per backend.

/// One detector under test, with a way to change what the instrument itself
/// is doing behind the program's back.
struct Rig {
    name: &'static str,
    mcb: AnyMcb,
    bench: Option<Shared>,
}

impl Rig {
    fn sim() -> Self {
        Self {
            name: "simulator",
            mcb: SimulatedMcb::new(1, "SIM").into(),
            bench: None,
        }
    }

    fn remote() -> Self {
        let bench = make_bench(|_| {});
        let mcb = remote_on(&bench).into();
        Self {
            name: "remote",
            mcb,
            bench: Some(bench),
        }
    }

    fn both() -> [Self; 2] {
        [Self::sim(), Self::remote()]
    }

    /// Makes the instrument count (the simulator through its own START; the
    /// network instrument at the far end only, the mirror left as it was).
    fn instrument_counts(&mut self, on: bool) {
        match &self.bench {
            Some(bench) => bench.lock().expect("bench").counting = on,
            None => {
                if on {
                    self.mcb.start().expect("start");
                } else {
                    self.mcb.stop().expect("stop");
                }
            }
        }
    }

    fn log(&self) -> Vec<String> {
        self.bench.as_ref().map(log).unwrap_or_default()
    }

    fn clear_log(&self) {
        if let Some(bench) = &self.bench {
            clear_log(bench);
        }
    }

    fn sets(&self) -> Vec<String> {
        self.bench.as_ref().map(sets).unwrap_or_default()
    }
}

fn uncertainty() -> UncertaintyPreset {
    UncertaintyPreset {
        limit_percent: 1.0,
        low_channel: 1,
        high_channel: 4,
    }
}

fn mda() -> MdaPreset {
    MdaPreset {
        target: 5.0,
        region: mantaray_core::Roi::new(1, 4),
        efficiency: 0.1,
        yield_fraction: 0.9,
    }
}

fn is_locked(error: &DeviceError) -> bool {
    matches!(error, DeviceError::Locked { .. })
}

fn is_out_of_range(error: &DeviceError) -> bool {
    matches!(error, DeviceError::OutOfRange { .. })
}

fn cannot_tell(error: &DeviceError) -> bool {
    error
        .to_string()
        .contains("cannot tell whether the detector is counting")
}

// ---------------------------------------------------------------------------
// Gate and lock.

/// T01 (both): a locked detector refuses every changing command, with nothing
/// sent, not even a status read; a read-only raw command still passes.
#[test]
fn t01_a_locked_detector_refuses_every_command_and_sends_nothing() {
    for mut rig in Rig::both() {
        rig.mcb.lock("pw", "Ann").expect("lock");
        rig.clear_log();
        let name = rig.name;
        let mcb = &mut rig.mcb;
        let mirror = mcb.properties().clone();
        let mut host_only = *mcb.presets();
        host_only.uncertainty = Some(uncertainty());
        let mut bias = mirror.clone();
        bias.high_voltage.target_volts = 1200.0;
        let results: Vec<(&str, Result<(), DeviceError>)> = vec![
            ("start", mcb.start()),
            ("stop", mcb.stop()),
            ("clear", mcb.clear()),
            ("apply host-only", mcb.apply_presets(host_only)),
            ("apply held", mcb.apply_presets(Presets::live(60.0))),
            ("set_presets", mcb.set_presets(Presets::default())),
            ("set_properties bias", mcb.set_properties(bias)),
            ("set_high_voltage", mcb.set_high_voltage(1000.0)),
            ("set_coarse_gain", mcb.set_coarse_gain(10.0)),
            (
                "set_mode",
                mcb.set_mode(mantaray_device::AcquisitionMode::List),
            ),
            ("start_optimize", mcb.start_optimize()),
            ("pole zero", mcb.set_pole_zero_running(true)),
            ("store_spectrum", mcb.store_spectrum().map(drop)),
            ("erase_stored", mcb.erase_stored()),
            ("send START", mcb.send_message("START").map(drop)),
            ("send SET_HV", mcb.send_message("SET_HV 100").map(drop)),
        ];
        for (what, result) in results {
            let error = result.expect_err(&format!("{name}: {what} on a locked detector"));
            assert!(is_locked(&error), "{name}: {what}: {error}");
            assert_eq!(error.to_string(), "the detector is locked by Ann");
        }
        assert_eq!(
            rig.log(),
            Vec::<String>::new(),
            "{name}: nothing at all sent"
        );
        assert_eq!(rig.mcb.properties(), &mirror, "{name}: nothing changed");
        let status = rig
            .mcb
            .send_message("SHOW_STATUS")
            .unwrap_or_else(|error| panic!("{name}: a read passes the lock: {error}"));
        assert!(status.contains("ACTIVE="), "{name}: {status}");
    }
}

/// T02 (both): a locked detector that reaches a preset is stopped anyway
/// (QUESTION 2), and the operator's own Stop on it is still refused.
#[test]
fn t02_a_locked_detector_is_stopped_at_its_preset_but_not_by_the_operator() {
    // Live preset.
    for mut rig in Rig::both() {
        let name = rig.name;
        rig.mcb.apply_presets(Presets::live(10.0)).expect("arm");
        rig.mcb.start().expect("start");
        rig.mcb.lock("pw", "Ann").expect("lock");
        assert!(is_locked(&rig.mcb.stop().expect_err("operator stop")));
        if let Some(bench) = &rig.bench {
            bench.lock().expect("bench").clock = 20.0;
        }
        let mut reached = None;
        for _ in 0..40 {
            match advance(&mut rig.mcb, 1.0) {
                Ok(Some(kind)) => {
                    reached = Some(kind);
                    break;
                }
                Ok(None) => {}
                Err(error) => panic!("{name}: advance failed: {error}"),
            }
        }
        assert_eq!(reached, Some(PresetKind::LiveTime), "{name}");
        assert!(!rig.mcb.is_active(), "{name}: stopped");
        if let Some(bench) = &rig.bench {
            assert!(
                !bench.lock().expect("bench").counting,
                "the instrument stopped"
            );
        }
    }
    // Uncertainty preset (host-side: only MantaRay can stop it).
    for mut rig in Rig::both() {
        let name = rig.name;
        // Around a peak: the bench's 8 channels, the simulator's Cs-137 line
        // (661.7 keV at 0.5 keV a channel).
        let (low_channel, high_channel) = match rig.bench {
            Some(_) => (0, 7),
            None => (1300, 1350),
        };
        let presets = Presets {
            uncertainty: Some(UncertaintyPreset {
                limit_percent: 5.0,
                low_channel,
                high_channel,
            }),
            ..Presets::default()
        };
        rig.mcb.apply_presets(presets).expect("arm");
        rig.mcb.start().expect("start");
        rig.mcb.lock("pw", "Ann").expect("lock");
        if let Some(bench) = &rig.bench {
            let mut b = bench.lock().expect("bench");
            b.data_always = Some(PEAK.into());
            b.clock = 5.0;
        }
        let mut reached = None;
        for _ in 0..200 {
            match advance(&mut rig.mcb, 1.0) {
                Ok(Some(kind)) => {
                    reached = Some(kind);
                    break;
                }
                Ok(None) => {}
                Err(error) => panic!("{name}: advance failed: {error}"),
            }
        }
        assert_eq!(reached, Some(PresetKind::Uncertainty), "{name}");
        assert!(!rig.mcb.is_active(), "{name}: stopped while locked");
        assert!(rig.mcb.is_locked(), "{name}: still locked");
    }
}

/// T03: unlocking with the wrong password is refused; the right one allows
/// commands again.
#[test]
fn t03_unlock_needs_the_right_password() {
    for mut rig in Rig::both() {
        rig.mcb.lock("pw", "Ann").expect("lock");
        assert!(is_locked(&rig.mcb.unlock("nope").expect_err("wrong")));
        assert!(is_locked(&rig.mcb.clear().expect_err("still locked")));
        rig.mcb.unlock("pw").expect("right");
        rig.mcb.clear().expect("allowed again");
    }
}

/// T04 (both): the order is locked, then counting, then values; the same
/// variant and text on both backends.
#[test]
fn t04_locked_then_counting_then_values_on_both_backends() {
    let invalid = Presets {
        live_time: Some(f64::NAN),
        ..Presets::default()
    };
    let mut texts: Vec<Vec<String>> = Vec::new();
    for mut rig in Rig::both() {
        let name = rig.name;
        let mut seen = Vec::new();
        // invalid alone
        let error = rig.mcb.apply_presets(invalid).expect_err("invalid");
        assert!(is_out_of_range(&error), "{name}: {error}");
        seen.push(error.to_string());
        // counting + invalid
        rig.instrument_counts(true);
        let error = rig.mcb.apply_presets(invalid).expect_err("counting");
        assert_eq!(error, DeviceError::PresetsWhileCounting, "{name}");
        seen.push(error.to_string());
        // locked + counting + invalid
        rig.mcb.lock("pw", "Ann").expect("lock");
        let error = rig.mcb.apply_presets(invalid).expect_err("locked");
        assert!(is_locked(&error), "{name}: {error}");
        seen.push(error.to_string());
        texts.push(seen);
    }
    assert_eq!(texts[0], texts[1], "the two backends say the same");
    assert_eq!(texts[0][1], "Presets can't be changed during acquisition.");
}

// ---------------------------------------------------------------------------
// Knowing whether it is counting.

/// T05 (both): the cache says idle and the instrument counts: every preset
/// change is refused, after one status read, with the mirror unchanged.
#[test]
fn t05_a_count_the_cache_missed_refuses_every_preset_change() {
    for mut rig in Rig::both() {
        let name = rig.name;
        rig.mcb.apply_presets(Presets::live(100.0)).expect("arm");
        rig.instrument_counts(true);
        let mirror = *rig.mcb.presets();
        let mut host_only = mirror;
        host_only.uncertainty = Some(uncertainty());
        let attempts: [(&str, Presets, bool); 3] = [
            ("held edit", Presets::live(60.0), false),
            ("host-only edit", host_only, false),
            ("clear presets", Presets::default(), true),
        ];
        for (what, presets, explicit) in attempts {
            rig.clear_log();
            let result = if explicit {
                rig.mcb.set_presets(presets)
            } else {
                rig.mcb.apply_presets(presets)
            };
            assert_eq!(
                result,
                Err(DeviceError::PresetsWhileCounting),
                "{name}: {what}"
            );
            if rig.bench.is_some() {
                assert_eq!(rig.log(), strs(&["SHOW_STATUS"]), "{name}: {what}");
            }
            assert_eq!(rig.mcb.presets(), &mirror, "{name}: {what}");
        }
    }
}

/// T06 (both): the cache says counting and the instrument has stopped itself
/// (a preset): the change is allowed and sends a clear and the held presets.
#[test]
fn t06_a_count_that_stopped_itself_allows_the_change() {
    for mut rig in Rig::both() {
        let name = rig.name;
        rig.mcb.start().expect("start");
        // A reading says it counts (an accepted START alone does not).
        rig.mcb.poll(1.0).expect("a reading");
        assert!(rig.mcb.is_active(), "{name}");
        match &rig.bench {
            Some(bench) => bench.lock().expect("bench").counting = false,
            None => {
                // The simulator stops itself only on a preset.
                rig.mcb.stop().expect("stop");
            }
        }
        rig.clear_log();
        rig.mcb
            .apply_presets(Presets::live(60.0))
            .unwrap_or_else(|error| panic!("{name}: {error}"));
        if rig.bench.is_some() {
            assert_eq!(
                rig.log(),
                strs(&["SHOW_STATUS", "SET_PRESET_CLEAR", "SET_PRESET_LIVE 60"]),
                "{name}"
            );
        }
        assert_eq!(rig.mcb.presets().live_time, Some(60.0), "{name}");
    }
}

/// T07 (with the amendment on odd status replies): a status line with no
/// ACTIVE word: a preset change is refused as unknown and nothing is sent; a
/// poll with that line is not an error - the reading is marked untrusted and
/// the flag stays as it was.
#[test]
fn t07_a_status_without_the_counting_flag_refuses_and_is_an_error_in_a_poll() {
    let bench = make_bench(|_| {});
    let mut remote = remote_on(&bench);
    bench.lock().expect("bench").status_always = Some("RT=1 LT=1".into());
    let error = remote
        .apply_presets(Presets::live(60.0))
        .expect_err("cannot tell");
    assert!(cannot_tell(&error), "{error}");
    assert_eq!(sets(&bench), Vec::<String>::new());
    assert_eq!(remote.presets().live_time, None);

    let before = remote.is_active();
    remote.poll(1.0).expect("the fetch goes on");
    assert!(remote.reading_untrusted().is_some());
    assert_eq!(remote.is_active(), before);
}

/// T08 (direct and courier): a poll moments old says idle and the instrument
/// now counts: the change is refused, and the status was read before
/// anything else went out. No road takes a poll's word.
#[test]
fn t08_a_recent_poll_is_never_the_answer() {
    for courier in [false, true] {
        for host_only in [false, true] {
            let bench = make_bench(|_| {});
            let mut remote = remote_on(&bench);
            if courier {
                remote = remote.with_courier();
                fetch_once(&mut remote);
            } else {
                remote.poll(1.0).expect("poll");
            }
            assert!(!remote.is_active());
            bench.lock().expect("bench").counting = true;
            clear_log(&bench);
            let mut presets = *remote.presets();
            if host_only {
                presets.uncertainty = Some(uncertainty());
            } else {
                presets.live_time = Some(60.0);
            }
            let result = if host_only {
                remote.apply_presets(presets)
            } else {
                remote.set_presets(presets)
            };
            assert_eq!(
                result,
                Err(DeviceError::PresetsWhileCounting),
                "courier={courier} host_only={host_only}"
            );
            let sent = log(&bench);
            assert_eq!(
                sent.first().map(String::as_str),
                Some("SHOW_STATUS"),
                "courier={courier} host_only={host_only}: {sent:?}"
            );
            assert!(sets(&bench).is_empty(), "{sent:?}");
        }
    }
}

/// Polls a courier-carried instrument until one fetch has been integrated,
/// leaving no other fetch in flight.
fn fetch_once(remote: &mut RemoteMcb) {
    let started = std::time::Instant::now();
    remote.poll(1.0).expect("request a fetch");
    loop {
        remote.poll(0.0).expect("poll");
        if remote.poll_freshened() {
            return;
        }
        assert!(started.elapsed() < Duration::from_secs(5), "no fetch");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// T09 (courier): a finished fetch that carries an error is waiting when a
/// preset change asks its own question: the change decides from its own read
/// and succeeds; the instrument's error shows at a later poll, not in apply.
#[test]
fn t09_an_unrelated_fetch_error_neither_fails_nor_vanishes_into_apply() {
    let bench = make_bench(|_| {});
    let mut remote = remote_on(&bench).with_courier();
    fetch_once(&mut remote);
    bench.lock().expect("bench").data_always = Some("ERR the memory is busy".into());
    // A fetch is asked for and completes into the slot.
    remote.poll(1.0).expect("request");
    std::thread::sleep(Duration::from_millis(100));
    remote
        .apply_presets(Presets::live(60.0))
        .expect("decided from its own read");
    assert_eq!(remote.presets().live_time, Some(60.0));
    let started = std::time::Instant::now();
    let error = loop {
        match remote.poll(1.0) {
            Err(error) => break error,
            Ok(()) => {
                assert!(started.elapsed() < Duration::from_secs(5), "never shown");
                std::thread::sleep(Duration::from_millis(5));
            }
        }
    };
    assert!(error.to_string().contains("memory is busy"), "{error}");
}

/// T10 + T47 (courier): a gate read neither integrates a waiting fetch nor
/// marks the poll fresh; the next poll asks for a fetch at once and works it
/// in normally.
#[test]
fn t10_t47_a_gate_read_touches_no_spectrum_and_the_next_poll_refetches() {
    let bench = make_bench(|b| b.counts = 1);
    let mut remote = remote_on(&bench).with_courier();
    fetch_once(&mut remote);
    assert_eq!(remote.spectrum().total_counts(), 8);
    // A newer fetch lands in the slot, not yet collected.
    bench.lock().expect("bench").counts = 2;
    remote.poll(1.0).expect("request");
    std::thread::sleep(Duration::from_millis(100));
    assert!(!remote.poll_freshened());
    let mut host_only = *remote.presets();
    host_only.mda = Some(mda());
    remote.apply_presets(host_only).expect("host-only edit");
    assert!(!remote.poll_freshened(), "a gate read is not a fetch");
    assert_eq!(
        remote.spectrum().total_counts(),
        8,
        "the waiting fetch was not worked in by the read"
    );
    let fetches_before = log(&bench).iter().filter(|c| *c == "SHOW_DATA").count();
    // No elapsed time: the next poll still asks for a fetch at once.
    remote.poll(0.0).expect("poll");
    let started = std::time::Instant::now();
    while log(&bench).iter().filter(|c| *c == "SHOW_DATA").count() == fetches_before {
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "the poll after a gate read did not refetch"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    let started = std::time::Instant::now();
    while !remote.poll_freshened() {
        remote.poll(0.0).expect("poll");
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "never integrated"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(remote.spectrum().total_counts(), 16, "integrated normally");
}

/// T11: a status read answered by anything but a status line refuses the
/// change as unknown, sends nothing and does not panic.
#[test]
fn t11_a_reply_that_is_not_a_status_refuses_the_change() {
    let junk = "x".repeat(5_000);
    for reply in [
        "$C04096106",
        "OK",
        "ERR busy",
        "DATA 8 0 0 0 0 0 0 0 0",
        "PRESETS REAL=0.00 LIVE=300.00 PEAK=0 INTEG=0",
        "",
        junk.as_str(),
    ] {
        let bench = make_bench(|_| {});
        let mut remote = remote_on(&bench);
        bench
            .lock()
            .expect("bench")
            .status_queue
            .push_back(reply.to_string());
        let error = remote
            .apply_presets(Presets::live(60.0))
            .expect_err("refused");
        assert!(cannot_tell(&error), "{:.40}: {error}", reply);
        assert_eq!(sets(&bench), Vec::<String>::new(), "{reply:.40}");
        assert_eq!(remote.presets().live_time, None);
    }
}

// ---------------------------------------------------------------------------
// Start and Stop.

/// T13: the instrument still says ACTIVE=1 on the first read after STOP and
/// 0 on the second: stop succeeds, and the next preset change too.
#[test]
fn t13_stop_waits_out_a_lagging_status() {
    let bench = make_bench(|b| b.stop_lag = 1);
    let mut remote = remote_on(&bench);
    remote.start().expect("start");
    remote.stop().expect("stopped once the instrument said so");
    remote
        .set_presets(Presets::live(60.0))
        .expect("the next change goes through");
}

/// T14: an idle instrument that answers ERR to STOP: stop succeeds (MAESTRO:
/// no operation). A counting one that answers ERR: stop fails with the ERR
/// text and the flag stays a reading (counting).
#[test]
fn t14_stop_succeeds_on_an_idle_instrument_whatever_it_answers() {
    let bench = make_bench(|b| {
        b.replies.insert("STOP".into(), "ERR not acquiring".into());
    });
    let mut remote = remote_on(&bench);
    remote.stop().expect("an idle detector is already stopped");

    let bench = make_bench(|b| {
        b.replies
            .insert("STOP".into(), "ERR the unit is busy".into());
    });
    let mut remote = remote_on(&bench);
    remote.start().expect("start");
    bench.lock().expect("bench").stop_works = false;
    let error = remote.stop().expect_err("still counting");
    assert!(error.to_string().contains("the unit is busy"), "{error}");
    assert!(remote.is_active(), "still counting, as read");
}

/// T15 (rule 9 as amended): a start a held preset already satisfies is
/// refused on a fresh reading and nothing else is sent; an accepted START
/// succeeds even if the detector is slow to say it counts (no confirm); an
/// ERR fails it.
#[test]
fn t15_start_is_decided_by_the_presets_and_the_reply() {
    let bench = make_bench(|b| {
        b.shown = Some("PRESETS REAL=0.00 LIVE=10.00 PEAK=0 INTEG=0".into());
        b.clock = 20.0;
    });
    let mut remote = remote_on(&bench);
    let error = remote.start().expect_err("already reached");
    assert!(
        error.to_string().contains("Preset already reached"),
        "{error}"
    );
    assert_eq!(log(&bench), strs(&["SHOW_STATUS"]));

    let bench = make_bench(|b| b.start_lag = 3);
    let mut remote = remote_on(&bench);
    remote.start().expect("accepted");
    assert_eq!(log(&bench), strs(&["SHOW_STATUS", "START"]));

    let bench = make_bench(|b| {
        b.replies.insert("START".into(), "ERR refused".into());
    });
    let mut remote = remote_on(&bench);
    let error = remote.start().expect_err("refused");
    assert!(error.to_string().contains("refused"), "{error}");
    assert!(!remote.is_active());
}

/// T12: the flag changes only on readings: a STOP answered OK over a count
/// that goes on is an error and the detector still reads as counting; a START
/// answered OK succeeds, but the mirror reads counting only once a reading
/// says so.
#[test]
fn t12_the_flag_is_only_ever_a_reading() {
    let bench = make_bench(|_| {});
    let mut remote = remote_on(&bench);
    remote.start().expect("start");
    bench.lock().expect("bench").stop_works = false;
    let error = remote.stop().expect_err("still counting");
    assert!(error.to_string().contains("did not stop"), "{error}");
    assert!(remote.is_active(), "not an invented false");

    let bench = make_bench(|b| b.start_counts = false);
    let mut remote = remote_on(&bench);
    remote.start().expect("an accepted START is a start");
    assert!(
        !remote.is_active(),
        "not an invented true: no reading said so"
    );
}

// ---------------------------------------------------------------------------
// Presets.

/// T19 (both, and again against an old bridge): the idle matrix, exact logs.
#[test]
fn t19_the_idle_matrix_sends_exactly_this() {
    for old_bridge in [false, true] {
        // The simulator has no wire; its outcome is the mirror.
        for mut rig in Rig::both() {
            let name = rig.name;
            if rig.bench.is_some() && old_bridge {
                let b = make_bench(|b| b.shown = None);
                rig.mcb = remote_on(&b).into();
                rig.bench = Some(b);
            }
            // (a) host-only change, mirror empty and then set.
            for start in [Presets::default(), Presets::live(300.0)] {
                if !start.is_empty() {
                    rig.mcb.apply_presets(start).expect("seed");
                }
                rig.clear_log();
                let mut edited = *rig.mcb.presets();
                edited.uncertainty = Some(uncertainty());
                rig.mcb.apply_presets(edited).expect("host only");
                if rig.bench.is_some() {
                    assert_eq!(rig.log(), strs(&["SHOW_STATUS"]), "{name} old={old_bridge}");
                }
                assert_eq!(rig.mcb.presets(), &edited, "{name}");
                rig.mcb.set_presets(Presets::default()).expect("reset");
            }
            // (b) held change.
            rig.clear_log();
            let held = Presets {
                real_time: Some(500.0),
                live_time: Some(300.0),
                roi_peak: Some(1000),
                roi_integral: Some(5000),
                ..Presets::default()
            };
            rig.mcb.apply_presets(held).expect("held");
            let full = strs(&[
                "SHOW_STATUS",
                "SET_PRESET_CLEAR",
                "SET_PRESET_REAL 500",
                "SET_PRESET_LIVE 300",
                "SET_PRESET_COUNT 1000",
                "SET_PRESET_INTEG 5000",
            ]);
            if rig.bench.is_some() {
                assert_eq!(rig.log(), full, "{name} old={old_bridge}");
            }
            assert_eq!(rig.mcb.presets(), &held);
            // (c) nothing changed: the same.
            rig.clear_log();
            rig.mcb.apply_presets(held).expect("re-apply");
            if rig.bench.is_some() {
                assert_eq!(rig.log(), full, "{name} old={old_bridge}");
            }
            // (d) clear presets, mirror empty.
            rig.mcb.set_presets(Presets::default()).expect("clear");
            rig.clear_log();
            rig.mcb
                .set_presets(Presets::default())
                .expect("clear again");
            if rig.bench.is_some() {
                assert_eq!(
                    rig.log(),
                    strs(&["SHOW_STATUS", "SET_PRESET_CLEAR"]),
                    "{name}"
                );
            }
            // (e) clear presets when only an MDA preset is held.
            let only_mda = Presets {
                mda: Some(mda()),
                ..Presets::default()
            };
            rig.mcb.apply_presets(only_mda).expect("mda");
            rig.clear_log();
            rig.mcb.set_presets(Presets::default()).expect("clear");
            if rig.bench.is_some() {
                assert_eq!(
                    rig.log(),
                    strs(&["SHOW_STATUS", "SET_PRESET_CLEAR"]),
                    "{name}"
                );
            }
            assert!(
                rig.mcb.presets().is_empty(),
                "{name}: the MDA preset is gone"
            );
        }
    }
}

/// T20: a batch sample whose presets equal the mirror still re-arms.
#[test]
fn t20_presets_equal_to_the_mirror_are_still_sent_by_set_presets() {
    let bench =
        make_bench(|b| b.shown = Some("PRESETS REAL=0.00 LIVE=300.00 PEAK=0 INTEG=0".into()));
    let mut remote = remote_on(&bench);
    remote.set_presets(Presets::live(300.0)).expect("re-arm");
    assert_eq!(
        sets(&bench),
        strs(&["SET_PRESET_CLEAR", "SET_PRESET_LIVE 300"])
    );
}

/// T21 (both): counting, and the same presets applied again: refused, and the
/// status read is the only thing on the wire.
#[test]
fn t21_re_applying_the_same_presets_mid_count_is_refused() {
    for mut rig in Rig::both() {
        let name = rig.name;
        rig.mcb.apply_presets(Presets::live(300.0)).expect("arm");
        rig.instrument_counts(true);
        rig.clear_log();
        let same = *rig.mcb.presets();
        assert_eq!(
            rig.mcb.apply_presets(same),
            Err(DeviceError::PresetsWhileCounting),
            "{name}"
        );
        if rig.bench.is_some() {
            assert_eq!(rig.log(), strs(&["SHOW_STATUS"]), "{name}");
        }
    }
}

/// T22: the mirror says 100 s, the front panel set 200 s: Apply of 100 s
/// sends the clear and 100 s (Apply never trusts the mirror to skip a send).
#[test]
fn t22_apply_reaches_an_instrument_changed_at_the_front_panel() {
    let bench = make_bench(|_| {});
    let mut remote = remote_on(&bench);
    remote.apply_presets(Presets::live(100.0)).expect("arm");
    // Somebody sets 200 s at the instrument; the mirror cannot know.
    clear_log(&bench);
    remote.apply_presets(Presets::live(100.0)).expect("apply");
    assert_eq!(
        sets(&bench),
        strs(&["SET_PRESET_CLEAR", "SET_PRESET_LIVE 100"])
    );
}

/// T23: a NaN or infinite preset is refused by name and nothing changes; an
/// instrument reporting such presets is read as holding none; a counting
/// detector still takes an amplifier edit.
#[test]
fn t23_a_preset_that_is_not_a_number_never_reaches_the_mirror() {
    for mut rig in Rig::both() {
        let name = rig.name;
        for bad in [f64::NAN, f64::INFINITY, -1.0] {
            let cases = [
                Presets {
                    real_time: Some(bad),
                    ..Presets::default()
                },
                Presets {
                    live_time: Some(bad),
                    ..Presets::default()
                },
                Presets {
                    uncertainty: Some(UncertaintyPreset {
                        limit_percent: bad,
                        ..uncertainty()
                    }),
                    ..Presets::default()
                },
                Presets {
                    mda: Some(MdaPreset {
                        target: bad,
                        ..mda()
                    }),
                    ..Presets::default()
                },
                Presets {
                    mda: Some(MdaPreset {
                        efficiency: bad,
                        ..mda()
                    }),
                    ..Presets::default()
                },
            ];
            for presets in cases {
                rig.clear_log();
                let error = rig.mcb.apply_presets(presets).expect_err("refused");
                assert!(is_out_of_range(&error), "{name}: {error}");
                assert!(rig.sets().is_empty(), "{name}");
                assert!(rig.mcb.presets().is_empty(), "{name}");
            }
        }
    }
    let bench = make_bench(|b| b.shown = Some("PRESETS REAL=NaN LIVE=inf PEAK=-5 INTEG=-1".into()));
    let remote = remote_on(&bench);
    assert!(remote.presets().is_empty(), "{:?}", remote.presets());

    for mut rig in Rig::both() {
        rig.instrument_counts(true);
        rig.clear_log();
        let mut properties = rig.mcb.properties().clone();
        properties.amplifier.rise_time_us = 4.0;
        rig.mcb
            .set_properties(properties)
            .expect("an amplifier edit");
        assert!(rig.sets().is_empty());
        assert_eq!(rig.mcb.properties().amplifier.rise_time_us, 4.0);
    }
}

/// T24 (both): settings never carry presets; bias and gain go out only when
/// they change; amplifier edits send nothing.
#[test]
fn t24_set_properties_never_changes_presets() {
    for mut rig in Rig::both() {
        let name = rig.name;
        let mut properties = rig.mcb.properties().clone();
        properties.presets.live_time = Some(60.0);
        let error = rig
            .mcb
            .set_properties(properties)
            .expect_err("presets go through Apply presets");
        assert!(
            error.to_string().contains("Apply presets"),
            "{name}: {error}"
        );
        assert!(rig.mcb.presets().is_empty(), "{name}");
        assert!(rig.log().is_empty(), "{name}: {:?}", rig.log());

        let mut properties = rig.mcb.properties().clone();
        properties.high_voltage.target_volts = 1200.0;
        rig.mcb.set_properties(properties).expect("bias");
        if rig.bench.is_some() {
            assert_eq!(rig.log(), strs(&["SET_HV 1200"]), "{name}");
        }
        rig.clear_log();
        let mut properties = rig.mcb.properties().clone();
        properties.amplifier.rise_time_us = 6.0;
        rig.mcb.set_properties(properties).expect("amplifier");
        assert!(rig.log().is_empty(), "{name}");
        let mut properties = rig.mcb.properties().clone();
        properties.amplifier.coarse_gain = 50.0;
        rig.mcb.set_properties(properties).expect("gain");
        if rig.bench.is_some() {
            assert_eq!(rig.log(), strs(&["SET_GAIN_COARSE 50"]), "{name}");
        }
        assert_eq!(rig.mcb.properties().amplifier.coarse_gain, 50.0);
    }
}

/// T25: zero is "not set", never a limit of zero; the time floor is the
/// instrument's tick; negative, NaN and infinite are refused.
#[test]
fn t25_zero_disables_and_the_tick_is_the_floor() {
    let bench = make_bench(|_| {});
    let mut remote = remote_on(&bench);
    remote
        .apply_presets(Presets {
            real_time: Some(0.0),
            ..Presets::default()
        })
        .expect("zero is off");
    assert_eq!(sets(&bench), strs(&["SET_PRESET_CLEAR"]));
    assert_eq!(remote.presets().real_time, None);
    clear_log(&bench);
    let error = remote
        .apply_presets(Presets::real(0.01))
        .expect_err("shorter than the clock's tick");
    assert!(is_out_of_range(&error), "{error}");
    assert!(sets(&bench).is_empty());
    remote.apply_presets(Presets::real(0.02)).expect("one tick");
    assert_eq!(remote.presets().real_time, Some(0.02));
    for mut rig in Rig::both() {
        rig.mcb
            .apply_presets(Presets {
                live_time: Some(0.0),
                roi_peak: Some(0),
                uncertainty: Some(UncertaintyPreset {
                    limit_percent: 0.0,
                    ..uncertainty()
                }),
                mda: Some(MdaPreset {
                    target: 0.0,
                    ..mda()
                }),
                ..Presets::default()
            })
            .expect("all off");
        assert!(rig.mcb.presets().is_empty(), "{:?}", rig.mcb.presets());
    }
}

/// T27 (both): an MDA efficiency or yield of zero, below zero or above one,
/// and an uncertainty range that runs backwards, are refused by name.
#[test]
fn t27_mda_fractions_and_channel_ranges_are_checked() {
    for mut rig in Rig::both() {
        let name = rig.name;
        for bad in [0.0, -0.1, 1.5, 85.0] {
            for (field, presets) in [
                (
                    "efficiency",
                    Presets {
                        mda: Some(MdaPreset {
                            efficiency: bad,
                            ..mda()
                        }),
                        ..Presets::default()
                    },
                ),
                (
                    "yield",
                    Presets {
                        mda: Some(MdaPreset {
                            yield_fraction: bad,
                            ..mda()
                        }),
                        ..Presets::default()
                    },
                ),
            ] {
                let error = rig.mcb.apply_presets(presets).expect_err("refused");
                assert!(is_out_of_range(&error), "{name}: {field} {bad}: {error}");
                assert!(error.to_string().contains(field), "{name}: {error}");
            }
        }
        rig.mcb
            .apply_presets(Presets {
                mda: Some(MdaPreset {
                    efficiency: 1.0,
                    yield_fraction: 1.0,
                    ..mda()
                }),
                ..Presets::default()
            })
            .expect("one is allowed");
        let error = rig
            .mcb
            .apply_presets(Presets {
                uncertainty: Some(UncertaintyPreset {
                    limit_percent: 5.0,
                    low_channel: 10,
                    high_channel: 5,
                }),
                ..Presets::default()
            })
            .expect_err("backwards");
        assert!(error.to_string().contains("backwards"), "{name}: {error}");
    }
}

/// T30: the second preset step refused: what applied and what was not sent
/// are named, the mirror says what the instrument was told, the host-side
/// presets are not stored, and a retry succeeds.
#[test]
fn t30_a_partial_preset_write_says_what_happened() {
    let bench = make_bench(|b| {
        b.replies
            .insert("SET_PRESET_REAL 500".into(), "ERR out of range".into());
    });
    let mut remote = remote_on(&bench);
    let wanted = Presets {
        real_time: Some(500.0),
        live_time: Some(300.0),
        uncertainty: Some(uncertainty()),
        ..Presets::default()
    };
    let error = remote.apply_presets(wanted).expect_err("partly applied");
    match &error {
        DeviceError::PartlyApplied { applied, failed } => {
            assert!(applied.contains("cleared"), "{applied}");
            assert!(failed.contains("real-time preset"), "{failed}");
            assert!(failed.contains("live-time preset"), "{failed}");
        }
        other => panic!("{other}"),
    }
    assert!(remote.presets().is_empty(), "{:?}", remote.presets());
    bench.lock().expect("bench").replies.clear();
    remote.apply_presets(wanted).expect("retry");
    assert_eq!(remote.presets(), &wanted);
}

// ---------------------------------------------------------------------------
// Host-side presets.

/// T33 (both): an uncertainty preset reached: STOP once per fresh reading,
/// never per frame; a STOP that fails is tried again at the next fresh
/// reading, with its error shown.
#[test]
fn t33_a_host_side_preset_stops_once_per_fresh_reading() {
    let bench = make_bench(|b| {
        b.replies
            .insert("STOP".into(), "ERR the unit is busy".into());
    });
    let mut remote = remote_on(&bench);
    remote
        .apply_presets(Presets {
            uncertainty: Some(UncertaintyPreset {
                limit_percent: 5.0,
                low_channel: 0,
                high_channel: 7,
            }),
            ..Presets::default()
        })
        .expect("arm");
    remote.start().expect("start");
    {
        let mut b = bench.lock().expect("bench");
        b.data_always = Some(PEAK.into());
        b.stop_works = false;
    }
    clear_log(&bench);
    let mut errors = 0;
    let mut fresh = 0;
    // Twenty frames of 0.1 s: a fetch every 0.5 s on the direct line.
    for _ in 0..20 {
        match advance(&mut remote, 0.1) {
            Err(error) => {
                errors += 1;
                assert!(error.to_string().contains("busy"), "{error}");
            }
            Ok(kind) => assert!(kind.is_none()),
        }
        if remote.poll_freshened() {
            fresh += 1;
        }
    }
    let stops = log(&bench).iter().filter(|c| *c == "STOP").count();
    assert!(fresh >= 3, "{fresh}");
    assert_eq!(stops, fresh, "one STOP per fresh reading");
    assert_eq!(errors, stops, "every failure shown");
    // The simulator: one stop, the run ends.
    let mut sim = SimulatedMcb::new(1, "SIM");
    sim.apply_presets(Presets {
        uncertainty: Some(UncertaintyPreset {
            limit_percent: 5.0,
            low_channel: 0,
            high_channel: 8191,
        }),
        ..Presets::default()
    })
    .expect("arm");
    sim.start().expect("start");
    let mut reached = None;
    for _ in 0..100 {
        if let Some(kind) = advance(&mut sim, 1.0).expect("advance") {
            reached = Some(kind);
            break;
        }
    }
    assert_eq!(reached, Some(PresetKind::Uncertainty));
    assert!(!sim.is_active());
}

/// T34 (both): a host-only edit while counting is refused; the mirror stays.
#[test]
fn t34_a_host_only_edit_mid_count_is_refused() {
    for mut rig in Rig::both() {
        rig.instrument_counts(true);
        let mut edited = *rig.mcb.presets();
        edited.mda = Some(mda());
        assert_eq!(
            rig.mcb.apply_presets(edited),
            Err(DeviceError::PresetsWhileCounting),
            "{}",
            rig.name
        );
        assert!(rig.mcb.presets().mda.is_none());
    }
}

// ---------------------------------------------------------------------------
// Idle behavior that must stay as it is.

/// T44: CLEAR zeroes the mirror's clocks, so the START after it is not
/// refused by a live preset the cleared count had reached.
#[test]
fn t44_clear_lets_the_next_start_through_a_reached_preset() {
    let bench = make_bench(|b| {
        b.shown = Some("PRESETS REAL=0.00 LIVE=10.00 PEAK=0 INTEG=0".into());
        b.clock = 20.0;
    });
    let mut remote = remote_on(&bench);
    assert!(remote.start().is_err(), "reached");
    remote.clear().expect("clear");
    remote.start().expect("starts after the clear");
}

/// T45: connecting asks SHOW_PRESETS once and adopts the leftovers; a bridge
/// that refuses the question connects with none.
#[test]
fn t45_connect_adopts_the_instruments_leftover_presets() {
    let bench =
        make_bench(|b| b.shown = Some("PRESETS REAL=0.00 LIVE=300.00 PEAK=0 INTEG=0".into()));
    let remote = RemoteMcb::connect(Box::new(Line(bench.clone())), 1, "B").expect("connect");
    assert_eq!(
        log(&bench).iter().filter(|c| *c == "SHOW_PRESETS").count(),
        1
    );
    assert_eq!(remote.presets().live_time, Some(300.0));
    let bench = make_bench(|b| b.shown = None);
    let remote = RemoteMcb::connect(Box::new(Line(bench.clone())), 1, "B").expect("connect");
    assert!(remote.presets().is_empty());
}

// ---------------------------------------------------------------------------
// The amendments after the review of 32c627e (device-spec.md, AMENDMENTS).

/// A network instrument on `bench` with the real confirm spacing.
fn remote_timed(bench: &Shared) -> RemoteMcb {
    let remote = RemoteMcb::connect(Box::new(Line(bench.clone())), 1, "BENCH").expect("connect");
    clear_log(bench);
    remote
}

/// Rule 9: an accepted START is a start, even for a count too short for any
/// read to see; one fresh status read comes first, and no read after.
#[test]
fn rule9_an_accepted_start_is_a_start_even_if_no_read_sees_it_count() {
    let bench = make_bench(|b| b.start_counts = false);
    let mut remote = remote_timed(&bench);
    remote.start().expect("accepted");
    assert_eq!(log(&bench), strs(&["SHOW_STATUS", "START"]));
    assert!(
        remote.spectrum().start_time.is_some(),
        "the start is stamped"
    );
}

/// Rule 9: an ERR to START fails at once, and never turns into a success
/// because the detector happens to be counting (another client's count).
#[test]
fn rule9_an_err_to_start_fails_at_once_even_while_the_detector_counts() {
    let bench = make_bench(|b| {
        b.replies
            .insert("START".into(), "ERR the memory is busy".into());
    });
    let mut remote = remote_timed(&bench);
    bench.lock().expect("bench").counting = true;
    let began = std::time::Instant::now();
    let error = remote.start().expect_err("the instrument said ERR");
    assert!(error.to_string().contains("the memory is busy"), "{error}");
    assert_eq!(log(&bench), strs(&["SHOW_STATUS", "START"]));
    assert!(began.elapsed() < Duration::from_millis(80), "no wait");
}

/// Rule 9: a start that a held preset already satisfies is refused before
/// anything is sent, with MAESTRO's words, on the clocks the detector
/// reports now (the count may have run elsewhere since the last poll).
#[test]
fn rule9_a_start_past_a_held_preset_is_refused_before_sending() {
    let bench = make_bench(|b| {
        b.shown = Some("PRESETS REAL=0.00 LIVE=10.00 PEAK=0 INTEG=0".into());
    });
    let mut remote = remote_on(&bench);
    bench.lock().expect("bench").clock = 20.0;
    let error = remote.start().expect_err("already reached");
    assert!(
        error.to_string().contains("Preset already reached"),
        "{error}"
    );
    assert_eq!(log(&bench), strs(&["SHOW_STATUS"]), "no START");

    let mut sim: AnyMcb = SimulatedMcb::new(1, "SIM").into();
    sim.apply_presets(Presets::live(2.0)).expect("arm");
    sim.start().expect("start");
    for _ in 0..10 {
        if advance(&mut sim, 1.0).expect("advance").is_some() || !sim.is_active() {
            break;
        }
    }
    assert!(!sim.is_active(), "the live preset stopped it");
    let error = sim.start().expect_err("already reached");
    assert!(
        error.to_string().contains("Preset already reached"),
        "{error}"
    );
    assert!(!sim.is_active());
}

/// Review 3: STOP's confirm is bounded, in reads (ten, 100 ms apart: about
/// a second), so a detector that never stops costs a bounded wait. The
/// bound is not in wall time (review of 21df1c3, #4: a slow adapter got two
/// reads where a fast one got ten), so it is checked by counting the reads.
#[test]
fn review3_a_stop_that_never_takes_is_refused_within_the_bound() {
    let bench = make_bench(|_| {});
    let mut remote = remote_timed(&bench);
    remote.start().expect("start");
    bench.lock().expect("bench").stop_works = false;
    let before = bench.lock().expect("bench").log.len();
    let began = std::time::Instant::now();
    let error = remote.stop().expect_err("still counting");
    assert!(error.to_string().contains("did not stop"), "{error}");
    let log = bench.lock().expect("bench").log.clone();
    let reads = log
        .iter()
        .skip(before)
        .filter(|command| command.as_str() == "SHOW_STATUS")
        .count();
    assert_eq!(reads, 10, "{log:?}");
    assert!(
        began.elapsed() < Duration::from_secs(3),
        "{:?}",
        began.elapsed()
    );
}

/// Review 2: a raw message is exactly one line; a second line (here a STOP
/// behind a read, past a lock) is refused and nothing is sent.
#[test]
fn review2_a_raw_message_is_one_line() {
    for locked in [false, true] {
        for mut rig in Rig::both() {
            let name = rig.name;
            if locked {
                rig.mcb.lock("pw", "Ann").expect("lock");
            }
            rig.clear_log();
            for message in ["SHOW_STATUS\r\nSTOP", "SHOW_STATUS\nSTOP", "STOP\r"] {
                let error = rig
                    .mcb
                    .send_message(message)
                    .expect_err(&format!("{name}: {message:?}"));
                assert!(error.to_string().contains("one line"), "{name}: {error}");
            }
            assert_eq!(rig.log(), Vec::<String>::new(), "{name}: nothing sent");
        }
    }
}

/// Review 6, and the review of 21df1c3 (#1): a raw SET_PRESET_* does not go
/// round the preset rules - it goes through them (the amended raw-message
/// rule allows either; through them is what lets `mantaray serve` pass a
/// client's presets on). Bad values and unknown preset verbs are refused
/// with nothing sent; a good one is a set_presets, mirror included.
#[test]
fn review6_a_raw_preset_command_goes_through_the_preset_rules() {
    for mut rig in Rig::both() {
        let name = rig.name;
        rig.mcb.apply_presets(Presets::live(300.0)).expect("seed");
        rig.clear_log();
        for message in [
            "set_preset_real NaN",
            "SET_PRESET_COUNT 1500.7",
            "SET_PRESET_COUNT 18446744073709551616",
            "SET_PRESET_COUNT 4.9999999999999999",
            "SET_PRESET_LIVE 300 600",
            "SET_PRESET_CLEAR 5",
            "SET_PRESET_LIVE",
            "SET_PRESET_UNCERTAINTY 5 10 20",
        ] {
            rig.mcb
                .send_message(message)
                .expect_err(&format!("{name}: {message}"));
        }
        assert!(
            !rig.log()
                .iter()
                .any(|command| command.starts_with("SET_PRESET")),
            "{name}: {:?}",
            rig.log()
        );
        assert_eq!(rig.mcb.presets().live_time, Some(300.0), "{name}");
        rig.mcb
            .send_message("SET_PRESET_LIVE 60")
            .unwrap_or_else(|error| panic!("{name}: {error}"));
        assert_eq!(rig.mcb.presets().live_time, Some(60.0), "{name}");
        rig.mcb
            .send_message("SET_PRESET_CLEAR")
            .unwrap_or_else(|error| panic!("{name}: {error}"));
        assert_eq!(rig.mcb.presets(), &Presets::default(), "{name}");
    }
}

/// Review of 45f02cf: a raw bias or gain is the same change as the typed
/// one, checked and mirrored, on every backend; one out of range is refused
/// before anything is sent.
#[test]
fn review_45f02cf_a_raw_bias_or_gain_goes_through_the_setting_rules() {
    for mut rig in Rig::both() {
        let name = rig.name;
        rig.clear_log();
        for message in [
            "SET_HV 99999",
            "SET_HV 1200.7",
            "SET_HV",
            "SET_GAIN_COARSE 1e9",
            // Round 2: no extra word takes a command round its typed door.
            "STOP 1",
            "CLEAR x",
            "START 0",
        ] {
            rig.mcb
                .send_message(message)
                .expect_err(&format!("{name}: {message}"));
        }
        assert!(
            !rig.log().iter().any(|command| {
                ["SET_HV", "SET_GAIN", "STOP", "CLEAR", "START"]
                    .iter()
                    .any(|verb| command.starts_with(verb))
            }),
            "{name}: {:?}",
            rig.log()
        );
        rig.mcb
            .send_message("SET_HV 1200")
            .unwrap_or_else(|error| panic!("{name}: {error}"));
        assert_eq!(
            rig.mcb.properties().high_voltage.target_volts,
            1200.0,
            "{name}: the mirror does not hold the bias that was sent"
        );
    }
}

/// Review of 45f02cf: a zero preset is the same as none (MAESTRO's zero
/// disables), so settings that carry one where the mirror holds none are
/// not a preset change and go through.
#[test]
fn review_45f02cf_a_zero_preset_in_the_settings_is_not_a_preset_change() {
    for mut rig in Rig::both() {
        let name = rig.name;
        let mut properties = rig.mcb.properties().clone();
        properties.presets.real_time = Some(0.0);
        properties.presets.roi_peak = Some(0);
        properties.high_voltage.target_volts = 1100.0;
        rig.mcb
            .set_properties(properties)
            .unwrap_or_else(|error| panic!("{name}: {error}"));
        assert_eq!(
            rig.mcb.properties().high_voltage.target_volts,
            1100.0,
            "{name}"
        );
    }
}

/// Review 7: a status reply that is not a status line keeps the last
/// displayed values and the spectrum fetch goes on; only changes are
/// refused, until a good reading comes back.
#[test]
fn review7_an_odd_status_keeps_the_display_and_the_fetch_goes_on() {
    let bench = make_bench(|b| {
        b.clock = 5.0;
        b.counts = 3;
    });
    let mut remote = remote_on(&bench);
    remote.poll(1.0).expect("a good poll");
    assert_eq!(remote.status().real_time, 5.0);
    for odd in [
        "ERR garbled",
        "RT=9.00 DT=0.0% TOTAL=8",
        "DATA 8 1 1 1 1 1 1 1 1",
    ] {
        {
            let mut b = bench.lock().expect("bench");
            b.status_always = Some(odd.into());
            b.counts += 4;
        }
        let counts = bench.lock().expect("bench").counts;
        remote
            .poll(1.0)
            .expect("an odd status is not a failed poll");
        assert_eq!(remote.status().real_time, 5.0, "{odd}: the last display");
        assert_eq!(
            remote.spectrum().channels[0],
            counts,
            "{odd}: the fetch went on"
        );
        assert!(remote.reading_untrusted().is_some(), "{odd}");
        clear_log(&bench);
        let error = remote
            .apply_presets(Presets::live(60.0))
            .expect_err("a change is refused");
        assert!(
            matches!(error, DeviceError::CountingUnknown { .. }),
            "{odd}: {error}"
        );
        assert_eq!(sets(&bench), Vec::<String>::new(), "{odd}");
    }
    bench.lock().expect("bench").status_always = None;
    remote.poll(1.0).expect("good again");
    assert!(remote.reading_untrusted().is_none());
    remote
        .apply_presets(Presets::live(60.0))
        .expect("changes go again");
}
