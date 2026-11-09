//! Running `.JOB` files and batch queues from the application.
//!
//! The job engine drives the application through [`JobHost`], so a job does
//! exactly what the menus do - including opening windows, so the operator can
//! watch it work. Commands are carried out a few per frame, which keeps the
//! display responsive during long jobs.

use std::path::{Path, PathBuf};

use mantaray_device::{DeviceError, JobState, Mcb, advance};
use mantaray_jobs::{BeepSpec, Job, JobError, JobHost, JobVariables, ListSlice, Runner};

use crate::app::{Action, App, Target};

/// A job in progress.
pub type JobRun = Runner;

/// How many commands are carried out per frame.
const COMMANDS_PER_FRAME: usize = 4;

/// A wait the job is in the middle of, checked once per frame.
///
/// A real instrument counts in real time, so its WAITs are wall-clock: looping
/// to completion inside one frame would freeze the interface for the whole
/// count while hammering the wire. Registering the wait here instead lets the
/// frame finish; the runner is simply not stepped again until the wait is
/// over. (The simulator runs faster than real time and never comes here - its
/// waits fast-forward it within the frame, as they always have.)
pub enum JobWait {
    /// WAIT: until the instrument stops itself (a preset is set). An
    /// accepted START is not yet a reading that it counts (rule 9), so the
    /// count is over at the first believed reading taken after the WAIT that
    /// says idle ([`mantaray_device::CountWatch`], the batch queue's rule
    /// too); readings that stay unbelievable stop the job with the reason.
    Stop {
        /// The detector's number (its identity, not its place in the list).
        detector: u16,
        /// The watch on its count.
        watch: mantaray_device::CountWatch,
    },
    /// WAIT n: until the wall clock reaches the deadline.
    Until(std::time::Instant),
    /// RUN_AND_WAIT: until the program exits.
    Program(String, std::process::Child),
    /// WAIT_OPTIMIZE: until the instrument stops optimizing.
    Optimize,
    /// Pole-zero WAIT: until the instrument stops pole-zeroing.
    PoleZero,
}

/// Whether the pending wait, if any, is over.
///
/// `Ok(true)` steps the job this frame, `Ok(false)` leaves it waiting, and
/// `Err` is a wait that ended badly and stops the job.
fn wait_over(app: &mut App) -> Result<bool, String> {
    let Some(mut wait) = app.job_wait.take() else {
        return Ok(true);
    };
    let done = match wait {
        JobWait::Stop {
            detector,
            ref mut watch,
        } => {
            let Some(mcb) = app
                .detectors
                .iter()
                .find(|mcb| mcb.identity().number == detector)
            else {
                return Err(format!("WAIT: detector {detector} is gone"));
            };
            match watch.check(mcb) {
                mantaray_device::CountState::Counting => false,
                mantaray_device::CountState::Over => true,
                mantaray_device::CountState::Unreadable(reason) => {
                    return Err(format!(
                        "WAIT: whether detector {detector} counts cannot be told: {reason}"
                    ));
                }
                mantaray_device::CountState::NeverStarted => {
                    return Err(format!(
                        "WAIT: detector {detector} accepted START but did not count"
                    ));
                }
            }
        }
        JobWait::Until(deadline) => std::time::Instant::now() >= deadline,
        JobWait::Program(ref program, ref mut child) => {
            let mut child_done = false;
            match child.try_wait() {
                Ok(None) => {}
                Ok(Some(status)) if status.success() => child_done = true,
                Ok(Some(status)) => return Err(format!("{program} exited with {status}")),
                Err(error) => return Err(format!("waiting on {program}: {error}")),
            }
            child_done
        }
        JobWait::Optimize => match app.job_target() {
            Some(detector) => !app.detectors[detector].optimizing(),
            None => true,
        },
        JobWait::PoleZero => match app.job_target() {
            Some(detector) => !app.detectors[detector].pole_zeroing(),
            None => true,
        },
    };
    if !done {
        app.job_wait = Some(wait);
    }
    Ok(done)
}

/// Starts a job file.
pub fn start(app: &mut App, path: &Path) -> Result<(), String> {
    let text = std::fs::read_to_string(path).map_err(|error| error.to_string())?;
    let job =
        Job::parse_named(&text, &path.display().to_string()).map_err(|error| error.to_string())?;
    app.job_directory = path
        .parent()
        .map(|parent| parent.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));
    app.job_variables.current_dir = app.job_directory.display().to_string();
    app.job = Some(Runner::new(job));
    // A fresh job must not inherit a dead one's pending wait, or its count.
    app.job_wait = None;
    app.job_count.clear();
    app.job_started_on = None;
    Ok(())
}

impl App {
    /// What a job's commands act on: the job's detector
    /// ([`App::job_detector`]), except that with a buffer window active it
    /// is the detector the job's last START counted on, when there was one.
    /// So a job's STOP and CLEAR act on what its START counted on, and a job
    /// never leaves a detector counting.
    fn job_target(&self) -> Option<usize> {
        if self.active_is_buffer()
            && let Some(number) = self.job_started_on
            && let Some(index) = self
                .detectors
                .iter()
                .position(|mcb| mcb.identity().number == number)
        {
            return Some(index);
        }
        self.job_detector()
    }

    /// A buffer window is active and the job has started no count: its STOP
    /// and CLEAR act on the buffer, as before (Gilder 11/7, QUESTIONS #9
    /// and #10), and no detector the job never started is touched.
    fn job_on_buffer_alone(&self) -> bool {
        self.active_is_buffer()
            && !self.job_started_on.is_some_and(|number| {
                self.detectors
                    .iter()
                    .any(|mcb| mcb.identity().number == number)
            })
    }
}

/// Carries out the next few commands of a running job.
pub fn step(app: &mut App) {
    if app.job.is_none() {
        // A cancelled job takes its pending wait with it.
        app.job_wait = None;
        return;
    }
    match wait_over(app) {
        Ok(true) => {}
        Ok(false) => return,
        Err(error) => {
            app.status = format!("job stopped: {error}");
            app.job = None;
            return;
        }
    }
    let Some(mut runner) = app.job.take() else {
        return;
    };
    for _ in 0..COMMANDS_PER_FRAME {
        match runner.step(app) {
            // A command that registered a wait ends the frame's batch: the
            // commands after it must not run until the wait is over.
            Ok(true) => {
                if app.job_wait.is_some() {
                    break;
                }
            }
            Ok(false) => {
                app.status = format!("job finished: {} command(s)", runner.outcome().commands);
                return;
            }
            Err(error) => {
                app.status = format!("job stopped: {error}");
                return;
            }
        }
    }
    app.job = Some(runner);
}

/// Counts the next queued sample on the active detector.
///
/// Straight-line, inside the click. First the checks that touch no
/// instrument, always in this order: a sample is waiting and not already
/// counting, the changer is not moving, a detector window is selected. Then
/// three steps, each through the detector's one gate: (1) the sample's
/// presets (refused if the detector is locked, counting - asked of it, once -
/// or the values are bad), (2) CLEAR, (3) START, confirmed by the detector.
///
/// It never stops a count: a detector still counting is refused with "stop it
/// first". A step that fails ends the attempt there and the message says
/// which steps were done; the sample stays waiting and pressing again repeats
/// every step. Nothing survives the click.
pub fn run_next_sample(app: &mut App) -> Vec<Action> {
    let status = |text: String| vec![Action::Status(text)];
    let Some(job) = app.queue.current().cloned() else {
        return status("the queue is empty".into());
    };
    if job.state != JobState::Pending {
        return status(format!("sample {} is already counting", job.id));
    }
    if app
        .changer
        .as_ref()
        .is_some_and(|changer| !changer.is_ready())
    {
        return status("the sample changer is still moving".into());
    }
    let Some(index) = app.active else {
        return status("open a detector window first".into());
    };
    let Target::Detector(detector) = app.windows[index].target else {
        return status("select a detector window".into());
    };
    let Some(mcb) = app.detectors.get_mut(detector) else {
        return status("that detector is gone".into());
    };
    // 1. Explicit: every sample re-arms the instrument, so a clear and every
    // held preset are sent even when they equal the last sample's.
    match mcb.set_presets(job.presets) {
        Ok(()) => {}
        Err(DeviceError::PresetsWhileCounting) => {
            return status(format!(
                "the detector is still counting; stop it first ({})",
                DeviceError::PresetsWhileCounting
            ));
        }
        Err(error) => return status(format!("{error}; sample {} was not started", job.id)),
    }
    let not_started = |applied: &str, step: &str, error: DeviceError| {
        DeviceError::PartlyApplied {
            applied: applied.to_string(),
            failed: format!("{step} failed: {error}; the sample was not started"),
        }
        .to_string()
    };
    // 2. A CLEAR that fails would count this sample on top of the last one's
    // spectrum: say so and go no further.
    if let Err(error) = mcb.clear() {
        return status(not_started(
            &format!("presets set for {}", job.id),
            "CLEAR",
            error,
        ));
    }
    // 3. START, which succeeds only when the detector says it counts.
    if let Err(error) = mcb.start() {
        return status(not_started(
            &format!("presets set for {}, spectrum cleared", job.id),
            "START",
            error,
        ));
    }
    let number = mcb.identity().number;
    let watch = mantaray_device::CountWatch::started(&*mcb);
    app.queue.begin_current();
    app.batch_count = Some(BatchCount {
        detector: number,
        watch,
    });
    app.sample.description = format!("{} ({})", job.id, job.description);
    vec![Action::Status(format!(
        "counting {} with {}",
        job.id,
        job.presets
            .live_time
            .map(|live| format!("{live:.0} s live"))
            .unwrap_or_else(|| "no preset".into())
    ))]
}

/// Finishes the sample being counted and advances the changer, if any.
pub fn finish_current_sample(app: &mut App) {
    app.queue.complete_current();
    app.batch_count = None;
    if let Some(changer) = &mut app.changer {
        changer.request_change();
    }
}

/// The queued sample being counted: which detector, and the watch on its
/// count.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BatchCount {
    /// The detector's number (its identity, not its place in the list).
    pub detector: u16,
    /// The count's end, judged on readings taken after its START.
    pub watch: mantaray_device::CountWatch,
}

/// Rule 10: watches the queued sample's count for its end, from the
/// readings ([`mantaray_device::CountWatch`], the same rule a job's WAIT
/// uses). When a reading taken after the START says idle, a reached preset -
/// one this program stopped it at, or one the instrument stopped itself on,
/// judged as the instrument judges its own clock ([`mantaray_device::Presets::ended_on`]) -
/// completes the sample and the changer moves; anything else (the
/// operator's Stop, another client's, a START that never counted) ends it
/// early, and the sample stays first in the queue to be counted again from
/// the start or skipped. Readings that stay unbelievable end it early too,
/// with the reason, rather than leaving it counting forever.
pub fn watch_batch(app: &mut App) {
    let Some(mut count) = app.batch_count else {
        return;
    };
    let Some(job) = app.queue.current().cloned() else {
        app.batch_count = None;
        return;
    };
    if job.state != JobState::Counting {
        app.batch_count = None;
        return;
    }
    let Some(mcb) = app
        .detectors
        .iter()
        .find(|detector| detector.identity().number == count.detector)
    else {
        stopped_early(app, &job.id, "its detector is gone");
        return;
    };
    match count.watch.check(mcb) {
        mantaray_device::CountState::Counting => {
            app.batch_count = Some(count);
            return;
        }
        mantaray_device::CountState::Unreadable(reason) => {
            stopped_early(
                app,
                &job.id,
                &format!("whether the detector counts cannot be told: {reason}"),
            );
            return;
        }
        mantaray_device::CountState::NeverStarted => {
            stopped_early(
                app,
                &job.id,
                "the detector accepted START but did not count",
            );
            return;
        }
        mantaray_device::CountState::Over => {}
    }
    let status = mcb.status();
    let Some(kind) = mcb
        .presets()
        .ended_on(mcb.spectrum(), status.real_time, status.live_time)
    else {
        stopped_early(app, &job.id, "it stopped before a preset was reached");
        return;
    };
    let number = count.detector;
    finish_current_sample(app);
    app.status = format!(
        "Detector {number} stopped: {} preset reached - sample complete ({}), {} left in the queue{}",
        kind.label(),
        job.id,
        app.queue.remaining(),
        if app.changer.is_none() && app.queue.remaining() > 0 {
            "; change the sample by hand"
        } else {
            ""
        }
    );
}

/// A count that ended before its preset: the sample waits again at the head
/// of the queue.
fn stopped_early(app: &mut App, id: &str, why: &str) {
    if let Some(job) = app.queue.current_mut() {
        job.state = JobState::Pending;
    }
    app.batch_count = None;
    app.status = format!(
        "sample {id} stopped early ({why}); it stays first in the queue - \
         Run next sample counts it again from the start, or Skip sample moves past it"
    );
}

/// Moves past the waiting sample at the head of the queue, marking it
/// skipped. A sample that is counting is not skipped: stop it first.
pub fn skip_sample(app: &mut App) -> Vec<Action> {
    let status = |text: String| vec![Action::Status(text)];
    let Some(job) = app.queue.current().cloned() else {
        return status("the queue is empty".into());
    };
    if job.state != JobState::Pending {
        return status(format!("sample {} is counting; stop it first", job.id));
    }
    app.queue.fail_current("skipped by the operator");
    status(format!("sample {} skipped", job.id))
}

/// The refusal for a changer command when there is no changer.
fn no_changer(command: &str) -> JobError {
    JobError::host(format!(
        "{command}: no sample changer is connected - change the sample by hand, \
         or run this job from a build that has one"
    ))
}

fn device(error: mantaray_device::DeviceError) -> JobError {
    JobError::host(error.to_string())
}

impl App {
    /// Resolves a job's file name against the job's directory.
    fn job_path(&self, path: &str) -> PathBuf {
        let candidate = Path::new(path);
        if candidate.is_absolute() {
            candidate.to_path_buf()
        } else {
            self.job_directory.join(candidate)
        }
    }

    /// Reads, changes and applies the job detector's presets through the one
    /// road every preset edit takes, so they reach the instrument: editing
    /// the mirror alone (`presets_mut`) would never arrive at a network
    /// instrument and would bypass the lock and the counting rules.
    fn change_presets(
        &mut self,
        change: impl FnOnce(&mut mantaray_device::Presets),
    ) -> Result<(), JobError> {
        let Some(detector) = self.job_target() else {
            return Err(JobError::host("no detector is configured"));
        };
        let mcb = &mut self.detectors[detector];
        let mut presets = *mcb.presets();
        change(&mut presets);
        mcb.apply_presets(presets).map_err(device)
    }
}

impl JobHost for App {
    fn variables(&mut self) -> &mut JobVariables {
        &mut self.job_variables
    }

    fn trace(&mut self, line: usize, command: &str) {
        self.status = format!("job line {line}: {command}");
    }

    fn select_detector(&mut self, number: Option<u16>) -> Result<(), JobError> {
        match number {
            Some(0) => {
                // Select the most recent buffer window, opening one if needed.
                match self.windows.iter().rposition(|window| window.is_buffer()) {
                    Some(index) => self.active = Some(index),
                    None => self.apply_action(Action::OpenBuffer),
                }
                Ok(())
            }
            Some(wanted) => {
                match self
                    .detectors
                    .iter()
                    .position(|mcb| mcb.identity().number == wanted)
                {
                    Some(index) => {
                        self.apply_action(Action::OpenDetector(index));
                        Ok(())
                    }
                    None => Err(JobError::host(format!("no detector numbered {wanted}"))),
                }
            }
            None => Ok(()),
        }
    }

    fn start(&mut self) -> Result<(), JobError> {
        // On the job's detector, and a refusal stops the job, as on the
        // command line: a job that went on to WAIT and SAVE after a START
        // that never happened would save an empty count as a measurement.
        let Some(detector) = self.job_target() else {
            return Err(JobError::host("START: there is no detector"));
        };
        let mcb = &mut self.detectors[detector];
        mcb.start().map_err(device)?;
        let number = mcb.identity().number;
        self.job_count
            .insert(number, mantaray_device::CountWatch::started(&*mcb));
        self.job_started_on = Some(number);
        self.status = "acquisition started".into();
        Ok(())
    }

    fn stop(&mut self) -> Result<(), JobError> {
        // On the job's detector, like START, and a STOP that did not take
        // stops the job: going on would CLEAR or SAVE a count still running.
        // The START it ends is over, so a later WAIT does not watch it.
        // With a buffer active this is the detector START counted on
        // ([`App::job_target`]), so a START/STOP job never leaves a count
        // running; with no START in the job, a buffer has no count to stop.
        if self.job_on_buffer_alone() {
            self.apply_action(Action::Stop);
            return Ok(());
        }
        let Some(detector) = self.job_target() else {
            return Err(JobError::host("STOP: there is no detector"));
        };
        self.detectors[detector].stop().map_err(device)?;
        let number = self.detectors[detector].identity().number;
        self.job_count.remove(&number);
        self.status = "acquisition stopped".into();
        Ok(())
    }

    fn clear(&mut self) -> Result<(), JobError> {
        // On what START counted on ([`App::job_target`]), even with a buffer
        // active; with no START in the job, the buffer, as before.
        if self.job_on_buffer_alone() {
            self.apply_action(Action::Clear);
            return Ok(());
        }
        // A detector that refused CLEAR still holds its counts; a START after
        // it would count on top of them, so the job stops here.
        let Some(detector) = self.job_target() else {
            return Err(JobError::host("CLEAR: there is no detector"));
        };
        // A count the job STARTed on this detector is still the one a later
        // WAIT watches (START / CLEAR / WAIT), its clocks from zero again;
        // the watch takes what the mirror holds before the CLEAR wipes it.
        let number = self.detectors[detector].identity().number;
        if let Some(watch) = self.job_count.get_mut(&number) {
            watch.clearing(&self.detectors[detector]);
        }
        self.clear_detector_for_job(detector).map_err(device)?;
        Ok(())
    }

    fn wait_for_stop(&mut self) -> Result<(), JobError> {
        let Some(detector) = self.job_target() else {
            return Ok(());
        };
        // This WAIT ends the watch on its detector's START, whichever way
        // it goes, so no later WAIT inherits it.
        let started = self
            .job_count
            .remove(&self.detectors[detector].identity().number);
        if self.detectors[detector].presets().is_empty() {
            return Err(JobError::host("WAIT with no preset set would never finish"));
        }
        if self.detectors[detector].as_simulated().is_some() {
            // The simulator runs far faster than real time; step it to the preset.
            let mut guard = 0usize;
            while self.detectors[detector].is_active() && guard < 2_000_000 {
                advance(&mut self.detectors[detector], 1.0).map_err(device)?;
                guard += 1;
            }
        } else {
            // A real instrument stops itself in real time; the job waits a
            // frame at a time, so the interface stays alive for the count.
            // It waits for a reading taken after this point even when the
            // mirror reads idle: right after an accepted START no reading has
            // yet said it counts. The watch the job's START began is used
            // when there is one for this detector, so a START that never
            // counts is told from a count that is over.
            let mcb = &self.detectors[detector];
            let number = mcb.identity().number;
            let watch = started.unwrap_or_else(|| mantaray_device::CountWatch::begin(mcb));
            self.job_wait = Some(JobWait::Stop {
                detector: number,
                watch,
            });
        }
        Ok(())
    }

    fn wait_seconds(&mut self, seconds: f64) -> Result<(), JobError> {
        if !seconds.is_finite() || seconds <= 0.0 {
            return Ok(());
        }
        let Some(detector) = self.job_target() else {
            return Ok(());
        };
        if self.detectors[detector].as_simulated().is_some() {
            if self.detectors[detector].is_active() {
                advance(&mut self.detectors[detector], seconds).map_err(device)?;
            }
        } else {
            // A real instrument counts in wall-clock time, so WAIT n means n
            // real seconds - fast-forwarding a remote mirror only bumps its
            // poll counter, and START / WAIT 300 / STOP saved a near-empty
            // spectrum.
            self.job_wait = Some(JobWait::Until(
                std::time::Instant::now() + std::time::Duration::from_secs_f64(seconds),
            ));
        }
        Ok(())
    }

    fn wait_changer(&mut self) -> Result<(), JobError> {
        let changer = self
            .changer
            .as_mut()
            .ok_or_else(|| no_changer("WAIT_CHANGER"))?;
        let mut guard = 0;
        while !changer.poll(1.0) && guard < 1_000 {
            guard += 1;
        }
        Ok(())
    }

    fn fill_buffer(&mut self) -> Result<(), JobError> {
        // The buffer is the job's other road from the mirror to a file: a
        // reading not believed is not copied either (review of the save-path
        // fixes, S-2).
        if let Some(reason) = self.active_untrusted() {
            return Err(JobError::host(crate::app::not_believed(&reason)));
        }
        self.apply_action(Action::CopyToBuffer);
        Ok(())
    }

    fn save(&mut self, path: Option<&str>) -> Result<(), JobError> {
        // Refused before either road: a bare SAVE goes through the window's
        // Save, which would only ask.
        if let Some(reason) = self.active_untrusted() {
            return Err(JobError::host(crate::app::not_believed(&reason)));
        }
        match path {
            Some(path) => {
                let target = self.job_path(path);
                self.save_active_to(&target).map_err(JobError::host)
            }
            None => {
                self.apply_action(Action::Save);
                Ok(())
            }
        }
    }

    fn save_rois(&mut self, path: &str) -> Result<(), JobError> {
        let target = self.job_path(path);
        let Some(spectrum) = self.active_spectrum() else {
            return Err(JobError::host("no spectrum to save regions from"));
        };
        mantaray_formats::save_rois(&spectrum.rois.clone(), &target)
            .map_err(|error| JobError::host(error.to_string()))
    }

    fn recall(&mut self, path: &str) -> Result<(), JobError> {
        let target = self.job_path(path);
        self.job_variables
            .set_spectrum_path(&target.display().to_string());
        self.recall_path(target);
        Ok(())
    }

    /// The same recall the menu does, through the same reader.
    ///
    /// A `.JOB` reaches this and so does `Calibrate / Recall Calibration`; when
    /// the two read files differently, automation quietly does something else
    /// than the operator watching it would. So both take a `.Clb` as well as a
    /// spectrum, and both refuse to overwrite a calibration in hand with a file
    /// that carries none.
    fn recall_calibration(&mut self, path: &str) -> Result<(), JobError> {
        let target = self.job_path(path);
        let (energy, shape) = mantaray_formats::load_calibration(&target)
            .map_err(|error| JobError::host(error.to_string()))?;
        self.job_variables
            .set_spectrum_path(&target.display().to_string());
        let Some(energy) = energy else {
            return Err(JobError::host(format!(
                "{} holds no calibration",
                target.display()
            )));
        };
        if let Some(spectrum) = self.active_spectrum_mut() {
            spectrum.energy_calibration = Some(energy);
            // Only when the file has one to give: a spectrum need not carry
            // `$SHAPE_CAL`, and assigning its absence would throw away the
            // peak widths the run is about to be measured with.
            if let Some(shape) = shape {
                spectrum.shape_calibration = Some(shape);
            }
        }
        Ok(())
    }

    fn recall_rois(&mut self, path: &str) -> Result<(), JobError> {
        let target = self.job_path(path);
        let rois = mantaray_formats::load_rois(&target)
            .map_err(|error| JobError::host(error.to_string()))?;
        self.job_variables
            .set_spectrum_path(&target.display().to_string());
        self.push_undo("Recall ROI File");
        if let Some(spectrum) = self.active_spectrum_mut() {
            spectrum.rois = rois;
        }
        Ok(())
    }

    fn strip(&mut self, factor: f64, path: Option<&str>) -> Result<(), JobError> {
        let name = path
            .map(|value| value.to_string())
            .or_else(|| self.strip_name.clone())
            .ok_or_else(|| JobError::host("STRIP without a file name"))?;
        let target = self.job_path(&name);
        self.job_variables
            .set_spectrum_path(&target.display().to_string());
        if self.strip_from(target, (factor != 0.0).then_some(factor)) {
            Ok(())
        } else {
            // strip_from wrote why into the status line.
            Err(JobError::host(self.status.clone()))
        }
    }

    fn set_strip_name(&mut self, path: &str) -> Result<(), JobError> {
        self.strip_name = Some(path.to_string());
        Ok(())
    }

    fn smooth(&mut self) -> Result<(), JobError> {
        self.apply_action(Action::Smooth);
        Ok(())
    }

    fn mark_peaks(&mut self) -> Result<(), JobError> {
        self.apply_action(Action::PeakSearch);
        Ok(())
    }

    fn report(&mut self, path: Option<&str>) -> Result<(), JobError> {
        self.apply_action(Action::RoiReport);
        let text = self.report_text.clone().unwrap_or_default();
        match path {
            // MAESTRO's printer device: show it on screen instead.
            Some(name) if name.eq_ignore_ascii_case("PRN") => Ok(()),
            Some(name) => {
                let target = self.job_path(name);
                mantaray_formats::write_atomic(target, text)
                    .map_err(|error| JobError::host(error.to_string()))
            }
            None => Ok(()),
        }
    }

    fn describe_sample(&mut self, description: &str) -> Result<(), JobError> {
        self.sample.description = description.to_string();
        let text = description.to_string();
        if let Some(spectrum) = self.active_spectrum_mut() {
            spectrum.sample_description = text;
        }
        Ok(())
    }

    fn load_library(&mut self, path: &str) -> Result<(), JobError> {
        let target = self.job_path(path);
        self.library = mantaray_formats::library::load(&target)
            .map_err(|error| JobError::host(error.to_string()))?;
        self.library_path = Some(target);
        Ok(())
    }

    fn run_program(&mut self, program: &str, minimized: bool) -> Result<(), JobError> {
        // Fire and forget, as MAESTRO's RUN does. "Minimized" is a window
        // arrangement the operating system no longer really honours; the
        // program still runs.
        let _ = minimized;
        crate::app::spawn_program(program)
            .map(drop)
            .map_err(JobError::host)
    }

    fn wait_program(&mut self, program: &str) -> Result<(), JobError> {
        // The program runs for as long as it runs; blocking the frame on it
        // froze the whole interface. The job pauses instead, and the child is
        // looked in on once per frame.
        let child = crate::app::spawn_program(program).map_err(JobError::host)?;
        self.job_wait = Some(JobWait::Program(program.to_string(), child));
        Ok(())
    }

    fn zoom(
        &mut self,
        rectangle: Option<(i32, i32, i32, i32)>,
        state: i32,
    ) -> Result<(), JobError> {
        match rectangle {
            Some((x, y, width, height)) => {
                let index = self
                    .active
                    .ok_or_else(|| JobError::host("no window to place"))?;
                let id = self.windows[index].id;
                self.place_window(
                    id,
                    egui::Rect::from_min_size(
                        egui::Pos2::new(x as f32, y as f32),
                        egui::vec2(width.max(120) as f32, height.max(90) as f32),
                    ),
                );
            }
            None => {
                // The state codes are Windows' ShowWindow numbers: 3 maximises,
                // the rest restore.
                if state == 3 {
                    self.apply_action(Action::MaximizeActive);
                } else {
                    self.apply_action(Action::RestoreWindows);
                }
            }
        }
        Ok(())
    }

    fn start_optimize(&mut self) -> Result<(), JobError> {
        let detector = self
            .job_target()
            .ok_or_else(|| JobError::host("no detector is configured"))?;
        self.detectors[detector].start_optimize().map_err(device)
    }

    fn pole_zero(&mut self, start: bool) -> Result<(), JobError> {
        let detector = self
            .job_target()
            .ok_or_else(|| JobError::host("no detector is configured"))?;
        self.detectors[detector]
            .set_pole_zero_running(start)
            .map_err(device)
    }

    fn wait_optimize(&mut self) -> Result<(), JobError> {
        let detector = self
            .job_target()
            .ok_or_else(|| JobError::host("no detector is configured"))?;
        if self.detectors[detector].as_simulated().is_some() {
            let mut guard = 0;
            while self.detectors[detector].optimizing() && guard < 100_000 {
                advance(&mut self.detectors[detector], 0.5).map_err(device)?;
                guard += 1;
            }
        } else if self.detectors[detector].optimizing() {
            self.job_wait = Some(JobWait::Optimize);
        }
        Ok(())
    }

    fn wait_pole_zero(&mut self) -> Result<(), JobError> {
        let detector = self
            .job_target()
            .ok_or_else(|| JobError::host("no detector is configured"))?;
        if self.detectors[detector].as_simulated().is_some() {
            let mut guard = 0;
            while self.detectors[detector].pole_zeroing() && guard < 100_000 {
                advance(&mut self.detectors[detector], 0.5).map_err(device)?;
                guard += 1;
            }
        } else if self.detectors[detector].pole_zeroing() {
            self.job_wait = Some(JobWait::PoleZero);
        }
        Ok(())
    }

    fn view_stored(&mut self, index: usize) -> Result<(), JobError> {
        // The manual's VIEW is one-based; 0 means the live spectrum.
        let detector = self
            .job_target()
            .ok_or_else(|| JobError::host("no detector is configured"))?;
        if index == 0 {
            self.apply_action(Action::OpenDetector(detector));
            return Ok(());
        }
        let name = self.detectors[detector].identity().name.clone();
        let spectrum = self.detectors[detector]
            .stored_spectrum(index - 1)
            .cloned()
            .ok_or_else(|| {
                JobError::host(format!(
                    "the instrument holds {} stored spectra, not {index}",
                    self.detectors[detector].stored_spectra()
                ))
            })?;
        self.open_buffer(format!("{name} store {index}"), spectrum, None);
        Ok(())
    }

    fn stored_spectra(&mut self) -> Result<usize, JobError> {
        let detector = self
            .job_target()
            .ok_or_else(|| JobError::host("no detector is configured"))?;
        Ok(self.detectors[detector].stored_spectra())
    }

    fn set_preset_real(&mut self, seconds: f64) -> Result<(), JobError> {
        self.change_presets(|presets| presets.set_real_time(seconds))
    }

    fn set_preset_live(&mut self, seconds: f64) -> Result<(), JobError> {
        self.change_presets(|presets| presets.set_live_time(seconds))
    }

    fn set_preset_peak(&mut self, counts: u64) -> Result<(), JobError> {
        self.change_presets(|presets| presets.set_roi_peak(counts))
    }

    fn set_preset_integral(&mut self, counts: u64) -> Result<(), JobError> {
        self.change_presets(|presets| presets.set_roi_integral(counts))
    }

    fn set_preset_uncertainty(
        &mut self,
        limit: f64,
        low: usize,
        high: usize,
    ) -> Result<(), JobError> {
        self.change_presets(|presets| presets.set_uncertainty(limit, low, high))
    }

    fn clear_presets(&mut self) -> Result<(), JobError> {
        // Explicit: always sent, like the command line's.
        let Some(detector) = self.job_target() else {
            return Err(JobError::host("no detector is configured"));
        };
        self.detectors[detector]
            .set_presets(mantaray_device::Presets::default())
            .map_err(device)
    }

    fn set_list_mode(&mut self, list: bool) -> Result<(), JobError> {
        let mode = if list {
            mantaray_device::AcquisitionMode::List
        } else {
            mantaray_device::AcquisitionMode::Pha
        };
        let Some(detector) = self.job_target() else {
            return Err(JobError::host("no detector selected"));
        };
        self.detectors[detector].set_mode(mode).map_err(device)
    }

    fn set_list_range(&mut self, slice: &ListSlice) -> Result<(), JobError> {
        let Some(index) = self.active else {
            return Err(JobError::host("no window selected"));
        };
        let Some(list) = self.windows[index].list.clone() else {
            return Err(JobError::host("this window holds no list-mode data"));
        };
        let range = match slice {
            ListSlice::Relative { start, duration } => {
                mantaray_formats::ListRange::from_seconds(*start, *duration)
            }
            ListSlice::Absolute {
                date,
                time,
                duration,
            } => {
                let stamp = format!("{date} {time}");
                let parsed = ["%m/%d/%Y %H:%M:%S", "%Y-%m-%d %H:%M:%S"]
                    .iter()
                    .find_map(|format| chrono::NaiveDateTime::parse_from_str(&stamp, format).ok())
                    .ok_or_else(|| JobError::host(format!("cannot read the date {stamp:?}")))?;
                let start = match list.start_time {
                    Some(begin) => (parsed - begin).num_milliseconds() as f64 / 1000.0,
                    None => 0.0,
                };
                mantaray_formats::ListRange::from_seconds(start.max(0.0), *duration)
            }
        };
        let sliced = list
            .slice(range)
            .ok_or_else(|| JobError::host("that time slice is empty"))?;
        let title = format!(
            "Slice {:.0}-{:.0} s",
            range.start,
            range.start + range.duration
        );
        self.open_buffer(title, sliced, None);
        Ok(())
    }

    fn send_message(&mut self, command: &str) -> Result<String, JobError> {
        let Some(detector) = self.job_target() else {
            return Err(JobError::host("no detector selected"));
        };
        // A bare START, STOP or CLEAR never arrives here: the job runs its
        // own (mantaray_jobs::exec), so Undo and the count watch follow it.
        self.detectors[detector]
            .send_message(command)
            .map_err(device)
    }

    fn lock(&mut self, password: &str, owner: &str, _name: Option<&str>) -> Result<(), JobError> {
        let Some(detector) = self.job_target() else {
            return Err(JobError::host("no detector selected"));
        };
        self.detectors[detector]
            .lock(password, owner)
            .map_err(device)
    }

    fn unlock(&mut self, password: &str) -> Result<(), JobError> {
        let Some(detector) = self.job_target() else {
            return Err(JobError::host("no detector selected"));
        };
        self.detectors[detector].unlock(password).map_err(device)
    }

    fn ask_password(&mut self) -> Result<(String, String), JobError> {
        Ok((
            self.dialogs.lock_password.clone(),
            self.dialogs.lock_owner.clone(),
        ))
    }

    fn change_sample(&mut self) -> Result<(), JobError> {
        let changer = self
            .changer
            .as_mut()
            .ok_or_else(|| no_changer("CHANGE_SAMPLE"))?;
        changer.request_change();
        let mut guard = 0;
        while !changer.poll(1.0) && guard < 1_000 {
            guard += 1;
        }
        Ok(())
    }

    fn beep(&mut self, _spec: &BeepSpec) -> Result<(), JobError> {
        self.status = "beep".into();
        Ok(())
    }

    fn close_buffers(&mut self) -> Result<(), JobError> {
        self.windows.retain(|window| !window.is_buffer());
        self.active = (!self.windows.is_empty()).then(|| self.windows.len() - 1);
        Ok(())
    }

    fn close_detectors(&mut self) -> Result<(), JobError> {
        self.windows.retain(|window| window.is_buffer());
        self.active = (!self.windows.is_empty()).then(|| self.windows.len() - 1);
        Ok(())
    }

    fn export(&mut self, path: &str) -> Result<(), JobError> {
        let target = self.job_path(path);
        let Some(spectrum) = self.active_spectrum() else {
            return Err(JobError::host("nothing to export"));
        };
        let text = mantaray_formats::ascii::write(spectrum, &Default::default());
        mantaray_formats::write_atomic(target, text)
            .map_err(|error| JobError::host(error.to_string()))
    }

    fn import(&mut self, path: &str) -> Result<(), JobError> {
        self.recall(path)
    }
}
