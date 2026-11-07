//! The instrument-control rules (runs/fcef80f/device-spec.md, Part 1 and
//! its two amendments), run end to end: the real `RemoteMcb` against the
//! scripted rig instrument, on the direct line and over a courier.
//!
//! Each test names the rule it holds and, where the spec numbers one, its
//! T-number. "Log" is what the rig received after the test's setup.

use mantaray_device::rig::{Fault, Registers, Rig};
use mantaray_device::{Mcb, Presets, RemoteMcb, SimulatedMcb, TcpTransport, UncertaintyPreset};

const CHANNELS: usize = 64;

/// The Apply button's door. The Presets tab's Apply goes through this one
/// call and nothing else, so it is the single seam between these tests and
/// the method the code under test names it by.
fn apply(mcb: &mut dyn Mcb, presets: Presets) -> Result<(), mantaray_device::DeviceError> {
    mcb.apply_presets(presets)
}

/// Connected on the direct line, log emptied.
fn direct(rig: &Rig) -> RemoteMcb {
    let mcb = RemoteMcb::connect(rig.transport(), 1, "RIG").expect("connect");
    rig.take_sent();
    mcb
}

/// Connected and then carried by a courier, as the application holds it.
fn carried(rig: &Rig) -> RemoteMcb {
    let mcb = RemoteMcb::connect(rig.transport(), 1, "RIG")
        .expect("connect")
        .with_courier();
    rig.take_sent();
    mcb
}

/// Both lines, for the rules that must hold on either.
fn both(test: impl Fn(&Rig, RemoteMcb, &str)) {
    let rig = Rig::new(CHANNELS);
    let mcb = direct(&rig);
    test(&rig, mcb, "direct");
    let rig = Rig::new(CHANNELS);
    let mcb = carried(&rig);
    test(&rig, mcb, "courier");
}

/// Polls until a fetch taken after this call began has been worked into the
/// mirror, so the mirror shows the rig as the test left it.
///
/// Over a courier the poll that collects one fetch asks for the next, so the
/// first fetch collected here may have been taken before the caller's last
/// change to the rig; the second was asked for inside this call. On the
/// direct line each poll fetches, and two cost nothing. The loop's deadline
/// turns a hang into a failure; it decides nothing.
fn fresh(mcb: &mut RemoteMcb) {
    let _ = step_fresh(mcb, |mcb| {
        // An error here is a reading the test may want; the tests that care
        // look at the mirror or poll themselves.
        let _ = mcb.poll(0.5);
        Ok(None)
    });
}

/// The acquisition loop's step, run until it ran on a fresh reading taken
/// after this call began (see [`fresh`]), or until it stopped or failed.
fn advance_fresh(
    mcb: &mut RemoteMcb,
) -> Result<Option<mantaray_device::PresetKind>, mantaray_device::DeviceError> {
    step_fresh(mcb, |mcb| mantaray_device::advance(mcb, 0.5))
}

fn step_fresh(
    mcb: &mut RemoteMcb,
    step: impl Fn(
        &mut RemoteMcb,
    ) -> Result<Option<mantaray_device::PresetKind>, mantaray_device::DeviceError>,
) -> Result<Option<mantaray_device::PresetKind>, mantaray_device::DeviceError> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let mut freshenings = 0;
    loop {
        let outcome = step(mcb);
        if !matches!(outcome, Ok(None)) {
            return outcome;
        }
        if mcb.poll_freshened() {
            freshenings += 1;
            if freshenings == 2 {
                return outcome;
            }
        }
        assert!(
            std::time::Instant::now() < deadline,
            "no fetch was integrated"
        );
        std::thread::yield_now();
    }
}

/// The commands the rig received since the last call, without the regular
/// fetches (a SHOW_STATUS directly followed by SHOW_DATA): over a courier a
/// fetch asked for by the last poll can land between a test's commands.
fn commands(rig: &Rig) -> Vec<String> {
    let sent = rig.take_sent();
    let mut kept = Vec::new();
    let mut words = sent.iter().peekable();
    while let Some(command) = words.next() {
        if command == "SHOW_DATA" {
            continue;
        }
        if command == "SHOW_STATUS" && words.peek().is_some_and(|next| *next == "SHOW_DATA") {
            continue;
        }
        kept.push(command.clone());
    }
    kept
}

fn live(seconds: f64) -> Presets {
    Presets {
        live_time: Some(seconds),
        ..Presets::default()
    }
}

fn uncertainty(limit_percent: f64) -> Presets {
    Presets {
        uncertainty: Some(UncertaintyPreset {
            limit_percent,
            // The rig's peak sits at mid-scale on an empty background.
            low_channel: CHANNELS / 2 - 4,
            high_channel: CHANNELS / 2 + 4,
        }),
        ..Presets::default()
    }
}

fn says(result: Result<(), mantaray_device::DeviceError>, words: &str, context: &str) {
    match result {
        Ok(()) => panic!("{context}: succeeded, expected a refusal saying {words:?}"),
        Err(error) => assert!(
            error.to_string().contains(words),
            "{context}: {error} (expected {words:?})"
        ),
    }
}

/// No SET_PRESET_* ever reached a counting instrument.
fn no_violations(rig: &Rig, context: &str) {
    assert!(
        rig.violations().is_empty(),
        "{context}: presets were sent to a counting detector: {:?}",
        rig.violations()
    );
}

// ---------------------------------------------------------------- rules 1, 2

#[test]
fn rule2_t01_a_locked_detector_refuses_every_change_and_sends_nothing() {
    both(|rig, mut mcb, line| {
        mcb.lock("pw", "Ann").expect("lock");
        let mut bias = mcb.properties().clone();
        bias.high_voltage.target_volts += 100.0;
        let attempts: Vec<(&str, Result<(), mantaray_device::DeviceError>)> = vec![
            ("start", mcb.start()),
            ("stop", mcb.stop()),
            ("clear", mcb.clear()),
            ("apply held", apply(&mut mcb, live(100.0))),
            ("apply host-only", apply(&mut mcb, uncertainty(5.0))),
            ("set_presets", mcb.set_presets(Presets::default())),
            ("set_properties bias", mcb.set_properties(bias)),
            ("raw START", mcb.send_message("START").map(drop)),
            ("raw SET_HV", mcb.send_message("SET_HV 100").map(drop)),
        ];
        for (what, result) in attempts {
            says(result, "locked by Ann", &format!("{line}: {what}"));
        }
        assert_eq!(rig.take_sent(), Vec::<String>::new(), "{line}: log");
        // Viewing is always allowed.
        mcb.send_message("SHOW_STATUS")
            .unwrap_or_else(|error| panic!("{line}: a read on a locked detector: {error}"));
    });
}

#[test]
fn rule2_t02_a_locked_detector_is_still_stopped_at_a_host_side_preset() {
    both(|rig, mut mcb, line| {
        apply(&mut mcb, uncertainty(10.0)).expect("arm");
        mcb.start().expect("start");
        mcb.lock("pw", "Ann").expect("lock");
        rig.tick(1.0);
        rig.take_sent();
        let mut stopped = None;
        for _ in 0..5 {
            if let Some(kind) = advance_fresh(&mut mcb).expect("advance") {
                stopped = Some(kind);
                break;
            }
        }
        assert_eq!(
            stopped,
            Some(mantaray_device::PresetKind::Uncertainty),
            "{line}"
        );
        assert!(!rig.counting(), "{line}: the rig is still counting");
        assert!(rig.sent().iter().any(|command| command == "STOP"), "{line}");
        // The operator's own stop is still refused.
        rig.front_panel_start();
        says(mcb.stop(), "locked by Ann", line);
    });
}

#[test]
fn rule2_t03_unlock_needs_the_password() {
    let rig = Rig::new(CHANNELS);
    let mut mcb = direct(&rig);
    mcb.lock("pw", "Ann").expect("lock");
    says(mcb.unlock("wrong"), "locked by Ann", "wrong password");
    mcb.unlock("pw").expect("right password");
    mcb.start().expect("start after unlock");
}

#[test]
fn rule1_t04_locked_then_counting_then_values() {
    both(|rig, mut mcb, line| {
        let bad = live(-5.0);
        rig.front_panel_start();
        mcb.lock("pw", "Ann").expect("lock");
        says(
            apply(&mut mcb, bad),
            "locked by Ann",
            &format!("{line}: locked+counting+bad"),
        );
        mcb.unlock("pw").expect("unlock");
        says(
            apply(&mut mcb, bad),
            "Presets can't be changed during acquisition.",
            &format!("{line}: counting+bad"),
        );
        rig.front_panel_stop();
        says(
            apply(&mut mcb, bad),
            "out of range",
            &format!("{line}: bad alone"),
        );
        no_violations(rig, line);
    });
}

// ------------------------------------------------- rules 3, 4: asked, refused

#[test]
fn rule3_t05_a_count_started_behind_the_programs_back_blocks_preset_changes() {
    both(|rig, mut mcb, line| {
        rig.front_panel_start();
        for (what, presets) in [
            ("held", live(100.0)),
            ("host-only", uncertainty(5.0)),
            ("clear presets", Presets::default()),
        ] {
            says(
                apply(&mut mcb, presets),
                "Presets can't be changed during acquisition.",
                &format!("{line}: {what}"),
            );
            assert_eq!(
                rig.take_sent(),
                vec!["SHOW_STATUS"],
                "{line}: {what}: one status read and nothing else"
            );
        }
        assert_eq!(mcb.presets(), &Presets::default(), "{line}: mirror");
        no_violations(rig, line);
    });
}

#[test]
fn rule3_t06_a_count_that_stopped_itself_does_not_block_a_change() {
    both(|rig, mut mcb, line| {
        apply(&mut mcb, live(1.0)).expect("arm");
        mcb.start().expect("start");
        fresh(&mut mcb);
        assert!(mcb.is_active(), "{line}: the mirror saw the count");
        rig.tick(2.0); // the rig stops itself on its live preset
        assert!(!rig.counting());
        rig.take_sent();
        apply(&mut mcb, live(5.0)).unwrap_or_else(|error| panic!("{line}: {error}"));
        assert_eq!(
            commands(rig),
            vec!["SHOW_STATUS", "SET_PRESET_CLEAR", "SET_PRESET_LIVE 5"],
            "{line}"
        );
        assert_eq!(rig.registers().live, Some(5.0), "{line}");
    });
}

#[test]
fn rule4_t07_a_status_without_its_counting_flag_refuses_the_change() {
    both(|rig, mut mcb, line| {
        rig.fault(Fault::StatusWithout {
            key: "ACTIVE".into(),
            count: 1,
        });
        says(
            apply(&mut mcb, live(100.0)),
            "cannot tell whether the detector is counting",
            line,
        );
        assert_eq!(rig.take_sent(), vec!["SHOW_STATUS"], "{line}");
        assert_eq!(rig.registers(), Registers::default(), "{line}");
    });
}

#[test]
fn rule4_t11_a_status_read_answered_by_something_else_refuses_the_change() {
    let junk = "x".repeat(5000);
    for reply in [
        "$C04096106",
        "OK",
        "ERR busy",
        "DATA 4 0 0 0 0",
        "PRESETS REAL=0.00 LIVE=0.00 PEAK=0 INTEG=0",
        "",
        junk.as_str(),
    ] {
        both(|rig, mut mcb, line| {
            rig.fault(Fault::Reply {
                verb: "SHOW_STATUS".into(),
                reply: reply.into(),
                count: 1,
                execute: false,
            });
            says(
                apply(&mut mcb, live(100.0)),
                "cannot tell whether the detector is counting",
                &format!("{line}: {reply:.20?}"),
            );
            assert_eq!(rig.take_sent(), vec!["SHOW_STATUS"], "{line}: {reply:.20?}");
        });
    }
}

#[test]
fn rule4_a_slipped_reply_stream_never_reads_as_idle() {
    // H-1: the status read is answered with the reply meant for the command
    // before it. A counting instrument must not be taken for an idle one.
    both(|rig, mut mcb, line| {
        mcb.start().expect("start");
        rig.take_sent();
        rig.fault(Fault::Slip { count: 1 });
        says(
            apply(&mut mcb, live(100.0)),
            "cannot tell whether the detector is counting",
            line,
        );
        assert!(
            !rig.sent()
                .iter()
                .any(|command| command.starts_with("SET_PRESET")),
            "{line}: {:?}",
            rig.sent()
        );
        no_violations(rig, line);
    });
}

#[test]
fn rule3_t08_a_recent_poll_is_not_an_answer() {
    both(|rig, mut mcb, line| {
        fresh(&mut mcb);
        assert!(!mcb.is_active());
        rig.front_panel_start();
        rig.take_sent();
        says(
            mcb.set_presets(live(100.0)),
            "Presets can't be changed during acquisition.",
            line,
        );
        assert_eq!(rig.sent().first().map(String::as_str), Some("SHOW_STATUS"));
        no_violations(rig, line);
    });
}

// ------------------------------------------------------- rule 9: start, stop

#[test]
fn rule9_t12_stop_that_never_takes_is_an_error_and_the_flag_is_not_invented() {
    both(|rig, mut mcb, line| {
        mcb.start().expect("start");
        fresh(&mut mcb);
        rig.fault(Fault::StopLag { reads: 1000 });
        says(mcb.stop(), "did not stop", line);
        assert!(mcb.is_active(), "{line}: an invented 'stopped'");
    });
}

#[test]
fn rule9_t13_t40_a_lagging_flag_after_stop_is_waited_out() {
    both(|rig, mut mcb, line| {
        mcb.start().expect("start");
        rig.fault(Fault::StopLag { reads: 3 });
        mcb.stop().unwrap_or_else(|error| panic!("{line}: {error}"));
        assert!(!mcb.is_active(), "{line}");
        apply(&mut mcb, live(10.0)).unwrap_or_else(|error| panic!("{line}: next preset {error}"));
        no_violations(rig, line);
    });
}

#[test]
fn rule9_finding4_a_slow_adapter_still_gets_its_confirm_reads() {
    // review-21df1c3 #4: a 250 ms budget that includes the round trips. With
    // 120 ms per exchange and the flag lagging three reads, a budget in
    // wall time runs out; a bound in reads does not.
    let rig = Rig::new(CHANNELS);
    let mut mcb = direct(&rig);
    mcb.start().expect("start");
    rig.set_latency(std::time::Duration::from_millis(120));
    rig.fault(Fault::StopLag { reads: 3 });
    mcb.stop()
        .expect("a detector that drops ACTIVE in ~400 ms has stopped");
    assert!(!mcb.is_active());
}

#[test]
fn rule9_t14_stop_on_an_idle_detector_that_answers_err_succeeds() {
    both(|rig, mut mcb, line| {
        rig.fault(Fault::Reply {
            verb: "STOP".into(),
            reply: "ERR not acquiring".into(),
            count: 1,
            execute: true,
        });
        mcb.stop()
            .unwrap_or_else(|error| panic!("{line}: idle + ERR: {error}"));
        // Counting, and STOP refused outright: the ERR is the error.
        mcb.start().expect("start");
        fresh(&mut mcb);
        rig.err("STOP", "busy", 1);
        says(mcb.stop(), "busy", line);
        assert!(mcb.is_active(), "{line}");
        assert!(rig.counting());
    });
}

#[test]
fn rule9_amended_t15_start() {
    both(|rig, mut mcb, line| {
        // A held preset already reached: refused before anything is sent.
        apply(&mut mcb, live(1.0)).expect("arm");
        mcb.start().expect("start");
        rig.tick(2.0);
        fresh(&mut mcb);
        rig.take_sent();
        says(mcb.start(), "already reached", line);
        assert!(
            !rig.sent().iter().any(|command| command == "START"),
            "{line}: {:?}",
            rig.sent()
        );
        // The mirror can be behind: the clocks moved at the front panel.
        mcb.clear().expect("clear");
        rig.front_panel_start();
        rig.tick(2.0);
        rig.take_sent();
        says(
            mcb.start(),
            "already reached",
            &format!("{line}: fresh read"),
        );
        assert!(!rig.sent().iter().any(|command| command == "START"));
        // START answered ERR fails at once, with no status wait after it.
        mcb.clear().expect("clear");
        rig.err("START", "adapter busy", 1);
        rig.take_sent();
        says(mcb.start(), "adapter busy", line);
        let sent = rig.take_sent();
        assert_eq!(
            sent.last().map(String::as_str),
            Some("START"),
            "{line}: {sent:?}"
        );
        // Accepted: success, no confirm read after START.
        mcb.start()
            .unwrap_or_else(|error| panic!("{line}: {error}"));
        let sent = rig.take_sent();
        assert_eq!(
            sent.last().map(String::as_str),
            Some("START"),
            "{line}: {sent:?}"
        );
    });
}

#[test]
fn rule9_start_on_a_detector_locked_elsewhere_fails_at_once() {
    both(|rig, mut mcb, line| {
        rig.lock_elsewhere("MAESTRO");
        says(mcb.start(), "locked by MAESTRO", line);
        assert!(!rig.counting());
        says(
            apply(&mut mcb, live(10.0)),
            "locked by MAESTRO",
            &format!("{line}: presets"),
        );
        assert_eq!(mcb.presets(), &Presets::default(), "{line}: mirror moved");
    });
}

// --------------------------------------------------------- rules 5, 6, 7

#[test]
fn rule6_t25_t27_preset_values() {
    both(|rig, mut mcb, line| {
        let mut refused = vec![
            (
                "real 0.01",
                Presets {
                    real_time: Some(0.01),
                    ..Presets::default()
                },
            ),
            ("live -1", live(-1.0)),
            ("live NaN", live(f64::NAN)),
            ("live inf", live(f64::INFINITY)),
            (
                "uncertainty backwards",
                Presets {
                    uncertainty: Some(UncertaintyPreset {
                        limit_percent: 5.0,
                        low_channel: 10,
                        high_channel: 5,
                    }),
                    ..Presets::default()
                },
            ),
        ];
        for (fraction, what) in [(0.0, "efficiency 0"), (1.5, "efficiency 1.5")] {
            let presets = Presets {
                mda: Some(mantaray_device::MdaPreset {
                    target: 1.0,
                    region: mantaray_core::Roi::new(10, 20),
                    efficiency: fraction,
                    yield_fraction: 0.5,
                }),
                ..Presets::default()
            };
            refused.push((what, presets));
        }
        for (what, presets) in refused {
            says(
                apply(&mut mcb, presets),
                "out of range",
                &format!("{line}: {what}"),
            );
        }
        assert!(
            !rig.sent()
                .iter()
                .any(|command| command.starts_with("SET_PRESET")),
            "{line}: {:?}",
            rig.sent()
        );
        rig.take_sent();
        // The clock tick is the floor, and zero means "not set".
        apply(
            &mut mcb,
            Presets {
                real_time: Some(0.02),
                ..Presets::default()
            },
        )
        .unwrap_or_else(|error| panic!("{line}: 0.02 s: {error}"));
        rig.take_sent();
        apply(&mut mcb, live(0.0)).unwrap_or_else(|error| panic!("{line}: zero: {error}"));
        assert_eq!(
            rig.take_sent(),
            vec!["SHOW_STATUS", "SET_PRESET_CLEAR"],
            "{line}: zero is 'not set'"
        );
        assert_eq!(mcb.presets().live_time, None, "{line}");
    });
}

#[test]
fn rule7_t19_what_apply_sends_on_an_idle_detector() {
    both(|rig, mut mcb, line| {
        // (b) a held change: clear, then each held preset.
        apply(&mut mcb, live(100.0)).unwrap_or_else(|error| panic!("{line}: {error}"));
        assert_eq!(
            rig.take_sent(),
            vec!["SHOW_STATUS", "SET_PRESET_CLEAR", "SET_PRESET_LIVE 100"],
            "{line}: held"
        );
        // (c) nothing changed: the same.
        apply(&mut mcb, live(100.0)).unwrap_or_else(|error| panic!("{line}: {error}"));
        assert_eq!(
            rig.take_sent(),
            vec!["SHOW_STATUS", "SET_PRESET_CLEAR", "SET_PRESET_LIVE 100"],
            "{line}: unchanged"
        );
        // (a) host-only: stored, nothing sent past the status read.
        let mut host = live(100.0);
        host.uncertainty = uncertainty(5.0).uncertainty;
        apply(&mut mcb, host).unwrap_or_else(|error| panic!("{line}: {error}"));
        assert_eq!(rig.take_sent(), vec!["SHOW_STATUS"], "{line}: host-only");
        assert_eq!(mcb.presets(), &host, "{line}: host-only stored");
        // (d) Clear presets always sends the clear.
        mcb.set_presets(Presets::default())
            .unwrap_or_else(|error| panic!("{line}: {error}"));
        assert_eq!(
            rig.take_sent(),
            vec!["SHOW_STATUS", "SET_PRESET_CLEAR"],
            "{line}: clear presets"
        );
        assert_eq!(
            mcb.presets(),
            &Presets::default(),
            "{line}: (e) host-side gone"
        );
        assert_eq!(rig.registers(), Registers::default());
    });
}

#[test]
fn rule5_t21_reapplying_the_same_value_mid_count_is_refused() {
    both(|rig, mut mcb, line| {
        apply(&mut mcb, live(100.0)).expect("arm");
        mcb.start().expect("start");
        rig.take_sent();
        says(
            apply(&mut mcb, live(100.0)),
            "Presets can't be changed during acquisition.",
            line,
        );
        assert_eq!(rig.take_sent(), vec!["SHOW_STATUS"], "{line}");
        no_violations(rig, line);
    });
}

#[test]
fn rule7_t22_a_front_panel_preset_change_is_overwritten_by_apply() {
    both(|rig, mut mcb, line| {
        apply(&mut mcb, live(100.0)).expect("arm");
        rig.front_panel_presets(Registers {
            live: Some(200.0),
            ..Registers::default()
        });
        rig.take_sent();
        apply(&mut mcb, live(100.0)).unwrap_or_else(|error| panic!("{line}: {error}"));
        assert_eq!(rig.registers().live, Some(100.0), "{line}");
    });
}

#[test]
fn rule7_t24_set_properties_does_not_carry_presets() {
    both(|rig, mut mcb, line| {
        let mut with_presets = mcb.properties().clone();
        with_presets.presets = live(50.0);
        says(
            mcb.set_properties(with_presets),
            "presets",
            &format!("{line}: presets through properties"),
        );
        assert!(
            !rig.sent()
                .iter()
                .any(|command| command.starts_with("SET_PRESET")),
            "{line}: {:?}",
            rig.sent()
        );
        rig.take_sent();
        let mut bias = mcb.properties().clone();
        bias.high_voltage.target_volts = 1200.0;
        mcb.set_properties(bias)
            .unwrap_or_else(|error| panic!("{line}: {error}"));
        assert_eq!(rig.take_sent(), vec!["SET_HV 1200"], "{line}: bias only");
        let mut amplifier = mcb.properties().clone();
        amplifier.amplifier.rise_time_us += 1.0;
        mcb.set_properties(amplifier)
            .unwrap_or_else(|error| panic!("{line}: {error}"));
        assert_eq!(
            rig.take_sent(),
            Vec::<String>::new(),
            "{line}: amplifier only"
        );
    });
}

#[test]
fn rule7_t30_a_preset_step_that_fails_says_what_was_applied() {
    let rig = Rig::new(CHANNELS);
    let mut mcb = direct(&rig);
    rig.err("SET_PRESET_LIVE", "register busy", 1);
    let both_presets = Presets {
        real_time: Some(200.0),
        live_time: Some(100.0),
        ..Presets::default()
    };
    let error = apply(&mut mcb, both_presets).expect_err("the live step fails");
    let text = error.to_string();
    assert!(text.contains("register busy"), "{text}");
    assert!(
        text.contains("real"),
        "says the real preset was set: {text}"
    );
    // The mirror says what the instrument was told.
    assert_eq!(mcb.presets().real_time, rig.registers().real);
    assert_eq!(mcb.presets().live_time, rig.registers().live);
    apply(&mut mcb, both_presets).expect("a retry succeeds");
    assert_eq!(rig.registers().live, Some(100.0));
}

// ------------------------------------------------------- rule 8: host-side

#[test]
fn rule8_t33_a_host_side_preset_stops_once_per_fresh_reading() {
    both(|rig, mut mcb, line| {
        apply(&mut mcb, uncertainty(10.0)).expect("arm");
        mcb.start().expect("start");
        rig.tick(1.0);
        rig.err("STOP", "adapter busy", 1);
        rig.take_sent();
        let first = advance_fresh(&mut mcb);
        assert!(first.is_err(), "{line}: a failed stop is shown: {first:?}");
        // Frames without a fresh reading do not resend STOP. (Over a courier
        // a fetch already asked for may land during these frames; each such
        // frame may send one STOP, and no other frame may.)
        let mut freshened = 0;
        for _ in 0..5 {
            let _ = mantaray_device::advance(&mut mcb, 0.0);
            freshened += usize::from(mcb.poll_freshened());
        }
        let stops = rig
            .sent()
            .iter()
            .filter(|command| *command == "STOP")
            .count();
        assert!(
            (1..=1 + freshened).contains(&stops),
            "{line}: {stops} STOPs over {freshened} fresh frames: {:?}",
            rig.sent()
        );
        let _ = advance_fresh(&mut mcb);
        assert!(!rig.counting(), "{line}: retried at the next fresh reading");
    });
}

// ---------------------------------------------------- idle, connect, raw

#[test]
fn t44_clear_lets_the_next_start_through() {
    both(|rig, mut mcb, line| {
        apply(&mut mcb, live(1.0)).expect("arm");
        mcb.start().expect("start");
        rig.tick(2.0);
        fresh(&mut mcb);
        mcb.clear().expect("clear");
        mcb.start()
            .unwrap_or_else(|error| panic!("{line}: {error}"));
        assert!(rig.counting(), "{line}");
    });
}

#[test]
fn t45_connect_adopts_the_presets_the_instrument_holds() {
    let rig = Rig::new(CHANNELS);
    rig.front_panel_presets(Registers {
        live: Some(300.0),
        ..Registers::default()
    });
    let mcb = direct(&rig);
    assert_eq!(mcb.presets().live_time, Some(300.0));
    let rig = Rig::new(CHANNELS);
    rig.err("SHOW_PRESETS", "unknown command", 1);
    let mcb = direct(&rig);
    assert_eq!(mcb.presets(), &Presets::default());
}

#[test]
fn finding7_connect_refuses_an_adapter_that_never_gives_a_status() {
    let rig = Rig::new(CHANNELS);
    rig.fault(Fault::Reply {
        verb: "SHOW_STATUS".into(),
        reply: "OK".into(),
        count: 1000,
        execute: false,
    });
    assert!(
        RemoteMcb::connect(rig.transport(), 1, "RIG").is_err(),
        "connected with an invented all-zero status"
    );
}

#[test]
fn amendment_an_odd_status_line_keeps_the_last_values_and_the_fetch_goes_on() {
    both(|rig, mut mcb, line| {
        mcb.start().expect("start");
        rig.tick(3.0);
        fresh(&mut mcb);
        let before = mcb.status();
        assert!(before.real_time > 2.9, "{line}");
        rig.tick(1.0);
        rig.fault(Fault::StatusWithout {
            key: "RT".into(),
            count: 2,
        });
        fresh(&mut mcb);
        assert_eq!(
            mcb.status().real_time,
            before.real_time,
            "{line}: invented RT"
        );
        assert!(mcb.is_active(), "{line}: invented idle");
        assert!(
            mcb.spectrum().total_counts() > before.total_counts,
            "{line}: the spectrum fetch stopped"
        );
    });
}

#[test]
fn amendment_raw_messages() {
    both(|rig, mut mcb, line| {
        // One line only.
        assert!(
            mcb.send_message("STOP\r\nSET_HV 3000").is_err(),
            "{line}: two lines in one message"
        );
        assert!(!rig.sent().iter().any(|command| command.contains("SET_HV")));
        // A raw preset obeys the preset rules: idle, it lands in the mirror.
        mcb.send_message("SET_PRESET_LIVE 30")
            .unwrap_or_else(|error| panic!("{line}: {error}"));
        assert_eq!(mcb.presets().live_time, Some(30.0), "{line}: mirror");
        assert_eq!(rig.registers().live, Some(30.0), "{line}");
        // Counting, it is refused and not sent.
        rig.front_panel_start();
        assert!(mcb.send_message("SET_PRESET_LIVE 60").is_err(), "{line}");
        no_violations(rig, line);
    });
}

#[test]
fn finding1_presets_reach_a_served_simulator() {
    // `mantaray serve` puts a SimulatedMcb behind the wire; every command
    // reaches it through send_message.
    use std::sync::{Arc, Mutex};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let address = listener.local_addr().expect("address").to_string();
    let served: Arc<Mutex<SimulatedMcb>> = Arc::new(Mutex::new(SimulatedMcb::new(1, "FAR")));
    let behind = Arc::clone(&served);
    std::thread::spawn(move || {
        if let Ok((stream, _)) = listener.accept() {
            let _ = mantaray_device::server::serve_connection(stream, behind);
        }
    });
    let transport =
        TcpTransport::connect(&address, std::time::Duration::from_secs(5)).expect("tcp");
    let mut mcb = RemoteMcb::connect(Box::new(transport), 1, "FAR").expect("connect");
    apply(&mut mcb, live(42.0)).expect("presets over the served wire");
    assert_eq!(served.lock().expect("lock").presets().live_time, Some(42.0));
}

#[test]
fn the_rig_served_over_tcp_runs_the_same_rules() {
    let rig = Rig::new(CHANNELS);
    let address = rig.serve();
    let transport =
        TcpTransport::connect(&address, std::time::Duration::from_secs(5)).expect("tcp");
    let mut mcb = RemoteMcb::connect(Box::new(transport), 1, "RIG")
        .expect("connect")
        .with_courier();
    rig.take_sent();
    rig.front_panel_start();
    says(
        apply(&mut mcb, live(10.0)),
        "Presets can't be changed during acquisition.",
        "tcp",
    );
    no_violations(&rig, "tcp");
}
