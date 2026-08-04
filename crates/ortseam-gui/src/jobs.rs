//! Running `.JOB` files and batch queues from the application.
//!
//! The job engine drives the application through [`JobHost`], so a job does
//! exactly what the menus do - including opening windows, so the operator can
//! watch it work. Commands are carried out a few per frame, which keeps the
//! display responsive during long jobs.

use std::path::{Path, PathBuf};

use ortseam_device::{Mcb, advance};
use ortseam_jobs::{BeepSpec, Job, JobError, JobHost, JobVariables, ListSlice, Runner};

use crate::app::{Action, App, Target};

/// A job in progress.
pub type JobRun = Runner;

/// How many commands are carried out per frame.
const COMMANDS_PER_FRAME: usize = 4;

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
    Ok(())
}

/// Carries out the next few commands of a running job.
pub fn step(app: &mut App) {
    let Some(mut runner) = app.job.take() else {
        return;
    };
    for _ in 0..COMMANDS_PER_FRAME {
        match runner.step(app) {
            Ok(true) => {}
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
pub fn run_next_sample(app: &mut App) -> Vec<Action> {
    let Some(job) = app.queue.current().cloned() else {
        return vec![Action::Status("the queue is empty".into())];
    };
    if !app.changer.is_ready() {
        return vec![Action::Status("the sample changer is still moving".into())];
    }
    let Some(index) = app.active else {
        return vec![Action::Status("open a detector window first".into())];
    };
    let Target::Detector(detector) = app.windows[index].target else {
        return vec![Action::Status("select a detector window".into())];
    };
    let Some(mcb) = app.detectors.get_mut(detector) else {
        return vec![Action::Status("that detector is gone".into())];
    };
    let _ = mcb.stop();
    if let Err(error) = mcb.set_presets(job.presets) {
        return vec![Action::Status(error.to_string())];
    }
    let _ = mcb.clear();
    if let Err(error) = mcb.start() {
        return vec![Action::Status(error.to_string())];
    }
    app.queue.begin_current();
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

/// Finishes the sample being counted and advances the changer.
pub fn finish_current_sample(app: &mut App) {
    app.queue.complete_current();
    app.changer.request_change();
}

fn device(error: ortseam_device::DeviceError) -> JobError {
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
        self.apply_action(Action::Start);
        Ok(())
    }

    fn stop(&mut self) -> Result<(), JobError> {
        self.apply_action(Action::Stop);
        Ok(())
    }

    fn clear(&mut self) -> Result<(), JobError> {
        self.apply_action(Action::Clear);
        Ok(())
    }

    fn wait_for_stop(&mut self) -> Result<(), JobError> {
        let Some(detector) = self.job_detector() else {
            return Ok(());
        };
        if self.detectors[detector].presets().is_empty() {
            return Err(JobError::host("WAIT with no preset set would never finish"));
        }
        // The simulator runs far faster than real time; step it to the preset.
        let mut guard = 0usize;
        while self.detectors[detector].is_active() && guard < 2_000_000 {
            advance(&mut self.detectors[detector], 1.0).map_err(device)?;
            guard += 1;
        }
        Ok(())
    }

    fn wait_seconds(&mut self, seconds: f64) -> Result<(), JobError> {
        if let Some(detector) = self.job_detector()
            && self.detectors[detector].is_active()
        {
            advance(&mut self.detectors[detector], seconds).map_err(device)?;
        }
        Ok(())
    }

    fn wait_changer(&mut self) -> Result<(), JobError> {
        let mut guard = 0;
        while !self.changer.poll(1.0) && guard < 1_000 {
            guard += 1;
        }
        Ok(())
    }

    fn fill_buffer(&mut self) -> Result<(), JobError> {
        self.apply_action(Action::CopyToBuffer);
        Ok(())
    }

    fn save(&mut self, path: Option<&str>) -> Result<(), JobError> {
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
        ortseam_formats::save_rois(&spectrum.rois.clone(), &target)
            .map_err(|error| JobError::host(error.to_string()))
    }

    fn recall(&mut self, path: &str) -> Result<(), JobError> {
        let target = self.job_path(path);
        self.job_variables
            .set_spectrum_path(&target.display().to_string());
        self.recall_path(target);
        Ok(())
    }

    fn recall_calibration(&mut self, path: &str) -> Result<(), JobError> {
        let target = self.job_path(path);
        let other = ortseam_formats::load_spectrum(&target)
            .map_err(|error| JobError::host(error.to_string()))?;
        self.job_variables
            .set_spectrum_path(&target.display().to_string());
        let (energy, shape) = (other.energy_calibration, other.shape_calibration);
        if let Some(spectrum) = self.active_spectrum_mut() {
            spectrum.energy_calibration = energy;
            spectrum.shape_calibration = shape;
        }
        Ok(())
    }

    fn recall_rois(&mut self, path: &str) -> Result<(), JobError> {
        let target = self.job_path(path);
        let rois = ortseam_formats::load_rois(&target)
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
        self.strip_from(target, (factor != 0.0).then_some(factor));
        Ok(())
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
                std::fs::write(target, text).map_err(|error| JobError::host(error.to_string()))
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
        self.library = ortseam_formats::library::load(&target)
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
        let mut child = crate::app::spawn_program(program).map_err(JobError::host)?;
        let status = child
            .wait()
            .map_err(|error| JobError::host(error.to_string()))?;
        if status.success() {
            Ok(())
        } else {
            Err(JobError::host(format!("{program} exited with {status}")))
        }
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
            .job_detector()
            .ok_or_else(|| JobError::host("no detector is configured"))?;
        self.detectors[detector].start_optimize().map_err(device)
    }

    fn pole_zero(&mut self, start: bool) -> Result<(), JobError> {
        let detector = self
            .job_detector()
            .ok_or_else(|| JobError::host("no detector is configured"))?;
        self.detectors[detector]
            .set_pole_zero_running(start)
            .map_err(device)
    }

    fn wait_optimize(&mut self) -> Result<(), JobError> {
        let detector = self
            .job_detector()
            .ok_or_else(|| JobError::host("no detector is configured"))?;
        let mut guard = 0;
        while self.detectors[detector].optimizing() && guard < 100_000 {
            advance(&mut self.detectors[detector], 0.5).map_err(device)?;
            guard += 1;
        }
        Ok(())
    }

    fn wait_pole_zero(&mut self) -> Result<(), JobError> {
        let detector = self
            .job_detector()
            .ok_or_else(|| JobError::host("no detector is configured"))?;
        let mut guard = 0;
        while self.detectors[detector].pole_zeroing() && guard < 100_000 {
            advance(&mut self.detectors[detector], 0.5).map_err(device)?;
            guard += 1;
        }
        Ok(())
    }

    fn view_stored(&mut self, index: usize) -> Result<(), JobError> {
        // The manual's VIEW is one-based; 0 means the live spectrum.
        let detector = self
            .job_detector()
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
            .job_detector()
            .ok_or_else(|| JobError::host("no detector is configured"))?;
        Ok(self.detectors[detector].stored_spectra())
    }

    fn set_preset_real(&mut self, seconds: f64) -> Result<(), JobError> {
        self.with_job_detector(|mcb| mcb.presets_mut().real_time = Some(seconds))
    }

    fn set_preset_live(&mut self, seconds: f64) -> Result<(), JobError> {
        self.with_job_detector(|mcb| mcb.presets_mut().live_time = Some(seconds))
    }

    fn set_preset_peak(&mut self, counts: u64) -> Result<(), JobError> {
        self.with_job_detector(|mcb| mcb.presets_mut().roi_peak = Some(counts))
    }

    fn set_preset_integral(&mut self, counts: u64) -> Result<(), JobError> {
        self.with_job_detector(|mcb| mcb.presets_mut().roi_integral = Some(counts))
    }

    fn set_preset_uncertainty(
        &mut self,
        limit: f64,
        low: usize,
        high: usize,
    ) -> Result<(), JobError> {
        self.with_job_detector(|mcb| {
            mcb.presets_mut().uncertainty = Some(ortseam_device::UncertaintyPreset {
                limit_percent: limit,
                low_channel: low,
                high_channel: high,
            })
        })
    }

    fn clear_presets(&mut self) -> Result<(), JobError> {
        self.with_job_detector(|mcb| mcb.presets_mut().clear())
    }

    fn set_list_mode(&mut self, list: bool) -> Result<(), JobError> {
        let mode = if list {
            ortseam_device::AcquisitionMode::List
        } else {
            ortseam_device::AcquisitionMode::Pha
        };
        let Some(detector) = self.job_detector() else {
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
                ortseam_formats::ListRange::from_seconds(*start, *duration)
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
                ortseam_formats::ListRange::from_seconds(start.max(0.0), *duration)
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
        let Some(detector) = self.job_detector() else {
            return Err(JobError::host("no detector selected"));
        };
        self.detectors[detector]
            .send_message(command)
            .map_err(device)
    }

    fn lock(&mut self, password: &str, owner: &str, _name: Option<&str>) -> Result<(), JobError> {
        let Some(detector) = self.job_detector() else {
            return Err(JobError::host("no detector selected"));
        };
        self.detectors[detector]
            .lock(password, owner)
            .map_err(device)
    }

    fn unlock(&mut self, password: &str) -> Result<(), JobError> {
        let Some(detector) = self.job_detector() else {
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
        self.changer.request_change();
        let mut guard = 0;
        while !self.changer.poll(1.0) && guard < 1_000 {
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
        let text = ortseam_formats::ascii::write(spectrum, &Default::default());
        std::fs::write(target, text).map_err(|error| JobError::host(error.to_string()))
    }

    fn import(&mut self, path: &str) -> Result<(), JobError> {
        self.recall(path)
    }
}
