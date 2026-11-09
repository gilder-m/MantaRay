//! The deep save-path review of 59e3230 (runs/release-0.2.5/deep-forward.md
//! F1, F2) as a job file meets it: the application drives the rig over TCP
//! through its Network-detector path (a `RemoteMcb` on a courier) and a job
//! SAVEs what it counted.

use mantaray_device::rig::{Fault, Rig};
use mantaray_device::{DetectorEntry, DetectorKind};
use mantaray_gui::app::{Action, App};

const CHANNELS: usize = 64;
const RATE: f64 = 100.0;

fn rig_app() -> (App, Rig, usize) {
    let rig = Rig::new(CHANNELS);
    rig.set_rate(RATE);
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
    (app, rig, index)
}

fn temp(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("mantaray-save-path-{}-{name}", std::process::id()))
}

/// Runs `text` as a job; `each` runs once per frame with the rig, after the
/// job's step and before the frame's poll, and says how long the frame is.
fn run(app: &mut App, rig: &Rig, name: &str, text: &str, mut each: impl FnMut(&Rig) -> f64) {
    let path = temp(name);
    std::fs::write(&path, text).expect("write the job");
    mantaray_gui::jobs::start(app, &path).expect("the job should parse");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    loop {
        mantaray_gui::jobs::step(app);
        if app.job.is_none() {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the job hung: {}",
            app.status
        );
        let frame = each(rig);
        app.advance_by(frame);
    }
    let _ = std::fs::remove_file(&path);
}

/// F1, the reviewer's repro: SHOW_DATA answers ERR from LT 2 on. The idle
/// status beside a failed read ended the WAIT, and SAVE wrote LT 2 s / 199
/// counts of a count that ran 3 s / 300; the job said "finished". Now the
/// WAIT goes on over the failed reads, and once they have failed for as
/// long as a WAIT gives an unreadable status, the job stops and says why.
#[test]
fn f1_a_job_does_not_save_a_short_spectrum_when_data_reads_fail() {
    let (mut app, rig, _) = rig_app();
    let saved = temp("f1.Chn");
    let _ = std::fs::remove_file(&saved);
    let mut armed = false;
    run(
        &mut app,
        &rig,
        "f1.job",
        &format!(
            "SET_PRESET_LIVE 3\nCLEAR\nSTART\nWAIT\nSAVE \"{}\"\n",
            saved.display()
        ),
        |rig| {
            if !armed && rig.clocks().1 >= 2.0 - 1e-9 {
                armed = true;
                rig.fault(Fault::Reply {
                    verb: "SHOW_DATA".into(),
                    reply: "ERR the adapter sent 0 of 64 bytes from 0x0000".into(),
                    count: 100_000,
                    execute: true,
                });
            }
            std::thread::sleep(std::time::Duration::from_millis(30));
            rig.tick(0.5);
            0.5
        },
    );
    let status = app.status.clone();
    let wrote = saved.exists();
    let _ = std::fs::remove_file(&saved);
    assert!(armed, "the count never reached LT 2");
    assert!(
        !wrote,
        "a spectrum that was never read was saved; status: {status}"
    );
    assert!(
        status.contains("job stopped") && status.contains("spectrum"),
        "{status}"
    );
}

/// F2, the reviewer's repro (MAESTRO's own pattern): START / WAIT 2 / STOP
/// / SAVE. STOP was confirmed on the status alone, so SAVE wrote the fetch
/// before it: LT 1.54 s / 154 counts of a detector stopped at 2.00 s / 200.
#[test]
fn f2_a_job_stop_then_save_writes_the_count_the_detector_stopped_at() {
    let (mut app, rig, _) = rig_app();
    let saved = temp("f2.Spe");
    let _ = std::fs::remove_file(&saved);
    let step = 0.05;
    let mut last = std::time::Instant::now();
    run(
        &mut app,
        &rig,
        "f2.job",
        &format!("CLEAR\nSTART\nWAIT 2\nSTOP\nSAVE \"{}\"\n", saved.display()),
        |rig| {
            // The rig's clock runs in step with the wall clock: it advances by
            // the time that really passed, so a slow runner whose sleeps
            // overrun (macOS CI, 11/9) cannot leave it behind the job's WAIT.
            std::thread::sleep(std::time::Duration::from_secs_f64(step));
            let now = std::time::Instant::now();
            let dt = now.duration_since(last).as_secs_f64();
            last = now;
            rig.tick(dt);
            dt
        },
    );
    let status = app.status.clone();
    assert!(status.contains("job finished"), "{status}");
    assert!(!rig.counting());
    let file = mantaray_formats::load_spectrum(&saved).expect("read back");
    let _ = std::fs::remove_file(&saved);
    let (_, live) = rig.clocks();
    assert!(live > 1.0, "the rig hardly counted: {live}");
    assert!(
        (file.live_time - live).abs() <= 0.02
            && (file.total_counts() as f64 - RATE * live).abs() <= 2.0,
        "the file holds LT {} s / {} counts; the detector stopped at LT {live} s / {} counts",
        file.live_time,
        file.total_counts(),
        (RATE * live).floor()
    );
}

/// Frames until the open detector's reading is not believed (SHOW_DATA
/// answering ERR from now on).
fn until_untrusted(app: &mut App, rig: &Rig, detector: usize) {
    use mantaray_device::Mcb;
    rig.fault(Fault::Reply {
        verb: "SHOW_DATA".into(),
        reply: "ERR the adapter sent 0 of 64 bytes from 0x0000".into(),
        count: 100_000,
        execute: true,
    });
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while app.detectors[detector].reading_untrusted().is_none() {
        assert!(std::time::Instant::now() < deadline, "never untrusted");
        std::thread::sleep(std::time::Duration::from_millis(30));
        rig.tick(0.1);
        app.advance_by(0.5);
    }
}

/// Review of audit/release-fix-save S-2 (Lucius): a job's SAVE with no WAIT
/// for the count's end or STOP before it wrote a mirror that was not
/// believed, without a word. It is refused, and the job says why.
#[test]
fn s2_a_job_save_of_a_reading_not_believed_is_refused() {
    let (mut app, rig, detector) = rig_app();
    let saved = temp("s2-job.Chn");
    let _ = std::fs::remove_file(&saved);
    app.apply_action(Action::Start);
    until_untrusted(&mut app, &rig, detector);
    run(
        &mut app,
        &rig,
        "s2.job",
        &format!("SAVE \"{}\"\n", saved.display()),
        |_| 0.05,
    );
    let status = app.status.clone();
    let wrote = saved.exists();
    let _ = std::fs::remove_file(&saved);
    assert!(!wrote, "a reading not believed was saved; status: {status}");
    assert!(
        status.contains("job stopped") && status.contains("not believed"),
        "{status}"
    );
}

/// The same for FILL_BUFFER, the job's other road from the mirror to a file.
#[test]
fn s2_a_job_fill_buffer_of_a_reading_not_believed_is_refused() {
    let (mut app, rig, detector) = rig_app();
    let saved = temp("s2-buffer.Chn");
    let _ = std::fs::remove_file(&saved);
    app.apply_action(Action::Start);
    until_untrusted(&mut app, &rig, detector);
    run(
        &mut app,
        &rig,
        "s2-buffer.job",
        &format!("FILL_BUFFER\nSET_BUFFER\nSAVE \"{}\"\n", saved.display()),
        |_| 0.05,
    );
    let status = app.status.clone();
    let wrote = saved.exists();
    let _ = std::fs::remove_file(&saved);
    assert!(
        !wrote,
        "a copy of a reading not believed was saved; status: {status}"
    );
    assert!(status.contains("not believed"), "{status}");
}

/// A manual Save of a window whose reading is not believed asks first: the
/// first Save says so and writes nothing, a second Save writes the last good
/// reading and says that is what it is.
#[test]
fn s2_a_manual_save_of_a_reading_not_believed_asks_first() {
    let (mut app, rig, detector) = rig_app();
    let saved = temp("s2-manual.Chn");
    let _ = std::fs::remove_file(&saved);
    let window = app.active.expect("the detector window is open");
    app.windows[window].path = Some(saved.clone());
    app.apply_action(Action::Start);
    until_untrusted(&mut app, &rig, detector);

    app.apply_action(Action::Save);
    let first = app.status.clone();
    assert!(!saved.exists(), "saved without asking; status: {first}");
    assert!(
        first.contains("not believed") && first.contains("Save again"),
        "{first}"
    );

    app.apply_action(Action::Save);
    let second = app.status.clone();
    let wrote = saved.exists();
    let _ = std::fs::remove_file(&saved);
    assert!(wrote, "the confirmed save wrote nothing: {second}");
    assert!(second.contains("last good reading"), "{second}");
}
