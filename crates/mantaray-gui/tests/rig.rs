//! The instrument-control rules as the operator meets them, end to end: the
//! application connects to the scripted rig instrument over TCP through its
//! own Network-detector path (a `RemoteMcb` on a courier), and the tests
//! press its buttons and run its batch queue and job files.
//!
//! Rule and T-numbers are those of runs/fcef80f/device-spec.md; "finding N"
//! is review-21df1c3.md's numbering.

use mantaray_device::rig::{Fault, Registers, Rig};
use mantaray_device::{DetectorEntry, DetectorKind, JobState, Mcb, Presets, SampleJob};
use mantaray_gui::app::{Action, App};

const CHANNELS: usize = 64;

/// An application with the rig as an open, active detector window.
fn rig_app() -> (App, Rig, usize) {
    let rig = Rig::new(CHANNELS);
    let address = rig.serve();
    let mut app = App::headless();
    let index = app.detectors.len();
    app.add_detector(DetectorEntry {
        number: 7,
        name: "RIG".into(),
        model: "remote".into(),
        kind: DetectorKind::Network,
        channels: 0,
        description: address,
        serial: String::new(),
    });
    assert_eq!(app.detectors.len(), index + 1, "connect: {}", app.status);
    app.apply_action(Action::OpenDetector(index));
    rig.take_sent();
    (app, rig, index)
}

/// Frames until the detector's mirror holds a fetch taken after this call
/// began (the courier's first collected fetch may predate it). The
/// deadline only turns a hang into a failure.
fn settle(app: &mut App, index: usize) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let mut freshenings = 0;
    while freshenings < 2 {
        app.advance_by(0.5);
        if app.detectors[index].poll_freshened() {
            freshenings += 1;
        }
        assert!(std::time::Instant::now() < deadline, "no fetch landed");
        std::thread::yield_now();
    }
}

/// The commands the rig received since the last call, without the regular
/// fetches (a SHOW_STATUS directly followed by SHOW_DATA).
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

fn sample(id: &str, live: f64) -> SampleJob {
    SampleJob {
        id: id.into(),
        description: format!("sample {id}"),
        presets: Presets {
            live_time: Some(live),
            ..Presets::default()
        },
        quantity: 1.0,
        unit: "g".into(),
        state: JobState::Pending,
    }
}

fn run_next(app: &mut App) {
    let actions = mantaray_gui::jobs::run_next_sample(app);
    for action in actions {
        app.apply_action(action);
    }
}

fn head_state(app: &App) -> Option<JobState> {
    app.queue.current().map(|job| job.state.clone())
}

// ------------------------------------------------------ rule 11, Stop/Start

#[test]
fn rule11_t17_stop_on_a_locked_detector_shows_the_lock() {
    let (mut app, rig, index) = rig_app();
    app.detectors[index].lock("pw", "Ann").expect("lock");
    app.apply_action(Action::Stop);
    assert!(app.status.contains("locked by Ann"), "{}", app.status);
    assert_eq!(commands(&rig), Vec::<String>::new());
}

#[test]
fn rule11_a_stop_that_did_not_take_is_not_reported_as_stopped() {
    let (mut app, rig, index) = rig_app();
    app.apply_action(Action::Start);
    settle(&mut app, index);
    rig.fault(Fault::StopLag { reads: 1000 });
    app.apply_action(Action::Stop);
    assert!(
        !app.status.contains("stopped"),
        "the status says stopped while the detector counts: {}",
        app.status
    );
    assert!(app.status.contains("did not stop"), "{}", app.status);
}

// ------------------------------------------------------- rule 10, the batch

#[test]
fn rule10_t35_run_next_sample_on_an_idle_detector() {
    let (mut app, rig, _) = rig_app();
    app.queue.push(sample("S1", 300.0));
    run_next(&mut app);
    assert_eq!(
        commands(&rig),
        vec![
            "SHOW_STATUS",
            "SET_PRESET_CLEAR",
            "SET_PRESET_LIVE 300",
            "CLEAR",
            "SHOW_STATUS",
            "START"
        ],
        "a status read for the presets, presets, clear, then START behind the fresh read \
         amended rule 9 asks for (a held preset already reached), and no confirm after it"
    );
    assert_eq!(head_state(&app), Some(JobState::Counting));
    assert!(
        app.status.contains("counting S1 with 300 s live"),
        "{}",
        app.status
    );
    assert!(rig.counting());
}

#[test]
fn rule10_t36_a_counting_detector_is_refused_not_stopped() {
    let (mut app, rig, _) = rig_app();
    rig.front_panel_start();
    app.queue.push(sample("S1", 300.0));
    run_next(&mut app);
    assert!(app.status.contains("stop it first"), "{}", app.status);
    assert_eq!(commands(&rig), vec!["SHOW_STATUS"]);
    assert!(rig.counting(), "the running count was stopped");
    assert_eq!(head_state(&app), Some(JobState::Pending));
    assert!(rig.violations().is_empty());
}

#[test]
fn rule10_t37_pre_checks_touch_nothing() {
    // An empty queue.
    let (mut app, rig, index) = rig_app();
    run_next(&mut app);
    assert!(app.status.contains("queue is empty"), "{}", app.status);
    // A locked detector: not even a status read.
    app.queue.push(sample("S1", 300.0));
    app.detectors[index].lock("pw", "Ann").expect("lock");
    run_next(&mut app);
    assert!(app.status.contains("locked by Ann"), "{}", app.status);
    assert_eq!(commands(&rig), Vec::<String>::new());
    assert_eq!(head_state(&app), Some(JobState::Pending));
}

#[test]
fn rule10_t38_a_double_click_starts_once() {
    let (mut app, rig, _) = rig_app();
    app.queue.push(sample("S1", 300.0));
    app.queue.push(sample("S2", 300.0));
    run_next(&mut app);
    commands(&rig);
    run_next(&mut app);
    assert!(
        app.status.contains("already counting") || app.status.contains("is counting"),
        "{}",
        app.status
    );
    assert!(
        !commands(&rig)
            .iter()
            .any(|command| command == "START" || command == "CLEAR"),
        "the second click touched the instrument"
    );
}

#[test]
fn rule10_a_start_that_never_counts_leaves_the_sample_waiting() {
    let (mut app, rig, index) = rig_app();
    app.queue.push(sample("S1", 300.0));
    rig.fault(Fault::Reply {
        verb: "START".into(),
        reply: "OK".into(),
        count: 1,
        execute: false,
    });
    run_next(&mut app);
    assert_eq!(head_state(&app), Some(JobState::Counting), "{}", app.status);
    for _ in 0..mantaray_device::NEVER_STARTED_READINGS {
        settle(&mut app, index);
    }
    assert_eq!(head_state(&app), Some(JobState::Pending), "{}", app.status);
    assert!(app.status.contains("did not count"), "{}", app.status);
}

#[test]
fn rule10_t39_a_failed_step_says_what_was_done_and_starts_nothing() {
    let (mut app, rig, _) = rig_app();
    app.queue.push(sample("S1", 300.0));
    rig.err("CLEAR", "memory busy", 1);
    run_next(&mut app);
    assert!(app.status.contains("memory busy"), "{}", app.status);
    assert!(
        app.status.contains("not started") || app.status.contains("was not started"),
        "{}",
        app.status
    );
    assert!(!commands(&rig).iter().any(|command| command == "START"));
    assert_eq!(head_state(&app), Some(JobState::Pending));
    run_next(&mut app);
    assert_eq!(head_state(&app), Some(JobState::Counting), "{}", app.status);
}

#[test]
fn rule10_amended_t42_the_instrument_stopping_itself_completes_the_sample() {
    let (mut app, rig, index) = rig_app();
    app.queue.push(sample("S1", 2.0));
    app.queue.push(sample("S2", 2.0));
    run_next(&mut app);
    assert_eq!(head_state(&app), Some(JobState::Counting), "{}", app.status);
    settle(&mut app, index);
    rig.tick(3.0); // the 926 stops itself on its live preset
    assert!(!rig.counting());
    settle(&mut app, index);
    settle(&mut app, index);
    assert_eq!(
        app.queue.jobs().first().map(|job| job.state.clone()),
        Some(JobState::Complete),
        "{}",
        app.status
    );
    assert!(!rig.counting(), "nothing starts by itself");
}

#[test]
fn rule10_finding5_a_clock_read_a_tick_short_still_completes_the_sample() {
    let (mut app, rig, index) = rig_app();
    rig.fault(Fault::ClockShortfall { seconds: 0.01 });
    app.queue.push(sample("S1", 2.0));
    run_next(&mut app);
    settle(&mut app, index);
    rig.tick(3.0);
    assert_eq!(rig.clocks().1, 1.99);
    settle(&mut app, index);
    settle(&mut app, index);
    assert_eq!(
        app.queue.jobs().first().map(|job| job.state.clone()),
        Some(JobState::Complete),
        "LT=1.99 on a 2 s preset read as stopped early: {}",
        app.status
    );
}

#[test]
fn rule10_amended_the_operators_stop_leaves_the_sample_at_the_head() {
    let (mut app, rig, index) = rig_app();
    app.queue.push(sample("S1", 300.0));
    run_next(&mut app);
    settle(&mut app, index);
    rig.tick(1.0);
    app.apply_action(Action::Stop);
    settle(&mut app, index);
    assert_eq!(head_state(&app), Some(JobState::Pending), "{}", app.status);
    assert_eq!(
        app.queue.current().map(|job| job.id.clone()),
        Some("S1".into())
    );
    // Run next sample counts it again from scratch.
    commands(&rig);
    run_next(&mut app);
    let sent = commands(&rig);
    assert!(sent.contains(&"CLEAR".to_string()), "{sent:?}");
    assert_eq!(head_state(&app), Some(JobState::Counting), "{}", app.status);
}

#[test]
fn finding8_an_operator_message_is_not_overwritten_by_an_odd_reading() {
    let (mut app, rig, index) = rig_app();
    app.queue.push(sample("S1", 2.0));
    run_next(&mut app);
    settle(&mut app, index);
    rig.tick(3.0);
    settle(&mut app, index);
    settle(&mut app, index);
    let message = app.status.clone();
    assert!(message.contains("complete"), "{message}");
    rig.fault(Fault::StatusWithout {
        key: "ACTIVE".into(),
        count: 8,
    });
    // The odd reading is said once, when it begins...
    settle(&mut app, index);
    assert!(app.status.contains("last good reading"), "{}", app.status);
    // ...and what the operator is told afterwards stays on screen while it
    // lasts (it used to be overwritten every frame).
    run_next(&mut app);
    let told = app.status.clone();
    assert!(told.contains("queue is empty"), "{told}");
    for _ in 0..3 {
        settle(&mut app, index);
    }
    assert!(
        app.detectors[index].reading_untrusted().is_some(),
        "the fault ran out before the check"
    );
    assert_eq!(app.status, told, "the operator's message was replaced");
}

#[test]
fn finding9_a_preset_draft_does_not_outlive_a_count() {
    let (mut app, rig, index) = rig_app();
    app.dialogs.presets_edit.insert(
        7,
        Presets {
            live_time: Some(50.0),
            ..Presets::default()
        },
    );
    rig.front_panel_start();
    settle(&mut app, index);
    // Another client sets presets while it counts, then the count ends.
    rig.front_panel_stop();
    rig.front_panel_presets(Registers {
        live: Some(900.0),
        ..Registers::default()
    });
    settle(&mut app, index);
    assert!(
        !app.dialogs.presets_edit.contains_key(&7),
        "the stale draft came back after the count: {:?}",
        app.dialogs.presets_edit.get(&7)
    );
}

// --------------------------------------------------------- job files' WAIT

fn run_job(app: &mut App, rig: &Rig, index: usize, text: &str, ticks: usize) -> String {
    let path = std::env::temp_dir().join(format!(
        "mantaray-rig-{}-{}.job",
        std::process::id(),
        text.len()
    ));
    std::fs::write(&path, text).expect("write the job");
    mantaray_gui::jobs::start(app, &path).expect("the job should parse");
    for _ in 0..ticks {
        mantaray_gui::jobs::step(app);
        if app.job.is_none() {
            break;
        }
        rig.tick(0.5);
        settle(app, index);
    }
    let _ = std::fs::remove_file(&path);
    app.status.clone()
}

#[test]
fn finding6_wait_on_an_idle_detector_returns_at_once() {
    let (mut app, rig, index) = rig_app();
    rig.front_panel_presets(Registers {
        live: Some(5.0),
        ..Registers::default()
    });
    settle(&mut app, index);
    let status = run_job(&mut app, &rig, index, "SET_PRESET_LIVE 5\nWAIT\n", 6);
    assert!(
        app.job.is_none(),
        "WAIT on an idle detector still waiting: {status}"
    );
}

#[test]
fn t16_start_then_wait_waits_for_the_real_stop() {
    let (mut app, rig, index) = rig_app();
    let status = run_job(
        &mut app,
        &rig,
        index,
        "SET_PRESET_LIVE 3\nSTART\nWAIT\n",
        40,
    );
    assert!(app.job.is_none(), "the job did not finish: {status}");
    assert!(status.contains("job finished"), "{status}");
    assert!(
        rig.clocks().1 >= 3.0 - 1e-9,
        "WAIT ended early: {:?}",
        rig.clocks()
    );
}

#[test]
fn finding3_wait_over_a_status_that_stays_unreadable_ends_with_an_error() {
    let (mut app, rig, index) = rig_app();
    let path = std::env::temp_dir().join(format!("mantaray-rig-{}-f3.job", std::process::id()));
    std::fs::write(&path, "SET_PRESET_LIVE 3\nSTART\nWAIT\n").expect("write the job");
    mantaray_gui::jobs::start(&mut app, &path).expect("the job should parse");
    // The presets and START go through; then the status stops being readable.
    mantaray_gui::jobs::step(&mut app);
    let _ = std::fs::remove_file(&path);
    assert!(rig.counting(), "the job did not start: {}", app.status);
    rig.fault(Fault::StatusWithout {
        key: "ACTIVE".into(),
        count: usize::MAX,
    });
    for _ in 0..200 {
        mantaray_gui::jobs::step(&mut app);
        if app.job.is_none() {
            break;
        }
        settle(&mut app, index);
    }
    assert!(
        app.job.is_none(),
        "WAIT spins forever on an unreadable status"
    );
    assert!(app.status.contains("job stopped"), "{}", app.status);
}

#[test]
fn rule3_a_front_panel_count_blocks_the_job_preset() {
    let (mut app, rig, index) = rig_app();
    rig.front_panel_start();
    settle(&mut app, index);
    let status = run_job(&mut app, &rig, index, "SET_PRESET_LIVE 3\n", 3);
    assert!(
        status.contains("Presets can't be changed during acquisition."),
        "{status}"
    );
    assert!(rig.violations().is_empty(), "{:?}", rig.violations());
    let _ = Mcb::presets(&app.detectors[index]);
}

// ------------------------------------------- review 45f02cf, job STOP/CLEAR

#[test]
fn review1_a_job_stop_that_did_not_take_stops_the_job() {
    let (mut app, rig, index) = rig_app();
    rig.fault(Fault::StopLag { reads: 1000 });
    let _ = commands(&rig);
    let status = run_job(
        &mut app,
        &rig,
        index,
        "SET_PRESET_LIVE 100\nSTART\nSTOP\nCLEAR\n",
        6,
    );
    assert!(app.job.is_none(), "the job went on: {status}");
    assert!(status.contains("did not stop"), "{status}");
    assert!(
        !commands(&rig).iter().any(|command| command == "CLEAR"),
        "the job cleared a detector that is still counting"
    );
}

#[test]
fn review1_a_job_clear_the_detector_refused_stops_the_job() {
    let (mut app, rig, index) = rig_app();
    rig.err("CLEAR", "busy", 1);
    let _ = commands(&rig);
    let status = run_job(
        &mut app,
        &rig,
        index,
        "CLEAR\nSET_PRESET_LIVE 3\nSTART\n",
        6,
    );
    assert!(app.job.is_none(), "the job went on: {status}");
    assert!(status.contains("job stopped"), "{status}");
    assert!(
        !commands(&rig).iter().any(|command| command == "START"),
        "the job started on top of a spectrum it could not clear"
    );
}

#[test]
fn review4_a_stopped_start_does_not_haunt_a_later_wait() {
    let (mut app, rig, index) = rig_app();
    // START then STOP before any reading saw the count, CLEAR, then a WAIT on
    // a count that is not the job's: the earlier START's watch is gone.
    let status = run_job(
        &mut app,
        &rig,
        index,
        "SET_PRESET_LIVE 3\nSTART\nSTOP\nCLEAR\nWAIT\n",
        12,
    );
    assert!(app.job.is_none(), "the job did not finish: {status}");
    assert!(status.contains("job finished"), "{status}");
}

#[test]
fn review_r2_a_job_stop_on_a_buffer_stops_no_detector() {
    // A job that started no count, with a buffer active: its STOP acts on
    // the buffer (nothing to stop), as on main (Gilder 11/7, #10: keep).
    let (mut app, rig, index) = rig_app();
    app.apply_action(Action::Start);
    settle(&mut app, index);
    assert!(rig.counting());
    app.apply_action(Action::CopyToBuffer);
    let _ = run_job(&mut app, &rig, index, "STOP\n", 3);
    assert!(
        rig.counting(),
        "a job STOP with a buffer active stopped a detector the job never started"
    );
    assert!(!app.detectors[0].is_active());
}

#[test]
fn review_r3_a_raw_stop_is_the_jobs_own_stop() {
    // SEND_MESSAGE "STOP" takes the job's door: with a buffer active and
    // no START in the job it stops nothing, as the job's STOP does not.
    let (mut app, rig, index) = rig_app();
    app.apply_action(Action::Start);
    settle(&mut app, index);
    assert!(rig.counting());
    app.apply_action(Action::CopyToBuffer);
    let _ = run_job(&mut app, &rig, index, "SEND_MESSAGE \"STOP\"\n", 3);
    assert!(
        rig.counting(),
        "a raw STOP with a buffer active stopped a detector the job never started"
    );
}

// ------------------------------------------- HIGH review of 05d1333

/// Runs a job on a headless application to its end.
fn run_headless_job(app: &mut App, name: &str, text: &str) {
    let job = std::env::temp_dir().join(format!("mantaray-{}-{name}.job", std::process::id()));
    std::fs::write(&job, text).expect("write the job");
    mantaray_gui::jobs::start(app, &job).expect("parse");
    for _ in 0..100 {
        mantaray_gui::jobs::step(app);
        if app.job.is_none() {
            break;
        }
    }
    let _ = std::fs::remove_file(&job);
    assert!(app.job.is_none(), "the job did not finish: {}", app.status);
}

#[test]
fn high_a_job_with_the_buffer_active_does_not_leave_a_detector_counting() {
    // A job's START, STOP and CLEAR act on one target, the job's detector:
    // with a buffer active that is the first detector, for all three.
    for stop in ["STOP", "SEND_MESSAGE \"STOP\""] {
        let mut app = App::headless();
        assert!(
            !app.detectors.is_empty(),
            "the simulator build has a detector"
        );
        app.open_buffer("Buffer".into(), mantaray_core::Spectrum::new(1024), None);
        assert!(!app.detectors[0].is_active());
        run_headless_job(&mut app, "buffer-stop", &format!("START\n{stop}\n"));
        assert!(
            !app.detectors[0].is_active(),
            "the job's START started detector {} and its {stop} left it counting; status: {}",
            app.detectors[0].identity().number,
            app.status
        );
    }
}

#[test]
fn high_a_job_clear_with_the_buffer_active_clears_what_start_counted() {
    for clear in ["CLEAR", "SEND_MESSAGE \"CLEAR\""] {
        let mut app = App::headless();
        let mut buffer = mantaray_core::Spectrum::new(1024);
        buffer.channels[10] = 42;
        app.open_buffer("Buffer".into(), buffer, None);
        run_headless_job(
            &mut app,
            "buffer-clear",
            &format!("SET_PRESET_LIVE 5\nSTART\nWAIT\n{clear}\n"),
        );
        assert_eq!(
            app.detectors[0].spectrum().total_counts(),
            0,
            "{clear} left the counts START made; status: {}",
            app.status
        );
        let buffer = app.active_spectrum().expect("the buffer is still active");
        assert_eq!(
            buffer.channels.get(10),
            Some(&42),
            "{clear} cleared the buffer, not the job's detector"
        );
    }
}

// ------------------------------------------- Gilder 11/7, a slow start

#[test]
fn gilder_a_saved_spectrum_is_dated_when_a_slow_count_began() {
    // The file's start time is when the detector began counting, not when
    // START was sent, for a detector slow to raise ACTIVE.
    let (mut app, rig, index) = rig_app();
    rig.fault(mantaray_device::rig::Fault::StartLag { reads: 10 });
    let sent = chrono::Local::now().naive_local();
    app.apply_action(Action::Start);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while !app.detectors[index].is_active() {
        std::thread::sleep(std::time::Duration::from_millis(150));
        app.advance_by(0.5);
        assert!(std::time::Instant::now() < deadline, "never seen counting");
    }
    let began = rig.began_at().expect("the rig began counting");
    assert!(
        (began - sent).num_milliseconds() >= 1200,
        "the rig did not lag long enough to tell ({sent} -> {began})"
    );
    let path = std::env::temp_dir().join(format!("mantaray-{}-slow-start.spe", std::process::id()));
    app.save_active_to(&path).expect("save");
    let saved = mantaray_formats::load_spectrum(&path).expect("read back");
    let _ = std::fs::remove_file(&path);
    let dated = saved.start_time.expect("the file carries a start time");
    // The file keeps whole seconds.
    assert!(
        (dated - began).num_milliseconds().abs() <= 1000 && dated > sent,
        "the file says {dated}; the count began {began}; START was sent {sent}"
    );
}

#[test]
fn high_review_a_stop_after_selecting_a_buffer_stops_what_start_counted_on() {
    // START on the rig, then SET_DETECTOR 0 makes a buffer active: the
    // job's STOP and CLEAR still act on the rig, never on the first
    // detector (the simulator here), which START did not count on.
    let (mut app, rig, index) = rig_app();
    let status = run_job(
        &mut app,
        &rig,
        index,
        "START\nSET_DETECTOR 0\nSTOP\nCLEAR\n",
        8,
    );
    assert!(app.job.is_none(), "the job did not finish: {status}");
    assert!(!rig.counting(), "the job left the rig counting: {status}");
    assert!(!app.detectors[0].is_active(), "{status}");
    let sent = commands(&rig);
    for verb in ["START", "STOP", "CLEAR"] {
        assert!(
            sent.iter().any(|command| command == verb),
            "{verb} did not reach the rig: {sent:?}"
        );
    }
}

#[test]
fn medium_review_a_buffer_clear_in_a_job_that_started_nothing_clears_the_buffer() {
    // Gilder 11/7, #9 "Yes": with a buffer active and no START in the job,
    // CLEAR (typed or raw) clears the buffer and touches no detector.
    for clear in ["CLEAR", "SEND_MESSAGE \"CLEAR\""] {
        let (mut app, rig, index) = rig_app();
        app.apply_action(Action::Start);
        settle(&mut app, index);
        rig.tick(2.0);
        settle(&mut app, index);
        app.apply_action(Action::CopyToBuffer);
        assert!(app.active_spectrum().is_some_and(|s| s.total_counts() > 0));
        let _ = commands(&rig);
        let status = run_job(&mut app, &rig, index, &format!("{clear}\n"), 3);
        assert_eq!(
            app.active_spectrum().map(|s| s.total_counts()),
            Some(0),
            "{clear} left the buffer full: {status}"
        );
        assert!(
            !commands(&rig).iter().any(|command| command == "CLEAR"),
            "{clear} cleared the rig the job never started"
        );
        assert!(rig.counting(), "{clear}");
    }
}
