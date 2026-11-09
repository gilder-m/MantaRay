//! A headless [`JobHost`]: runs `.JOB` files against an instrument and the file
//! system.
//!
//! The instrument is whatever the caller hands over - a network MCB on the
//! bench, or the simulator in a build that has one. A job that acquires needs
//! something to acquire from; there is no fallback that invents counts.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use mantaray_core::{
    CalculationSettings, NuclideLibrary, Spectrum, StripFactor, mark_peaks, smooth, strip,
};
use mantaray_device::{AnyMcb, Mcb, Presets, advance};
use mantaray_formats::{
    ListRange, SpectrumFormat, library, list_mode, load_rois, load_spectrum, save_rois,
};

use crate::save_spectrum;
use mantaray_jobs::{BeepSpec, Job, JobError, JobHost, JobVariables, ListSlice, run};
use mantaray_report::{ReportOptions, roi_report};

/// Which spectrum a job command applies to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Target {
    /// The buffer (detector 0).
    Buffer,
    /// The selected detector.
    Detector,
}

/// The state a job runs against.
pub struct CliSession {
    variables: JobVariables,
    directory: PathBuf,
    detector: Option<AnyMcb>,
    buffer: Spectrum,
    /// Stands in for the detector spectrum when nothing is attached.
    empty: Spectrum,
    buffer_path: Option<PathBuf>,
    target: Target,
    previous_target: Target,
    library: NuclideLibrary,
    settings: CalculationSettings,
    sample_description: String,
    strip_name: Option<String>,
    list_file: Option<mantaray_formats::ListModeFile>,
    trace: bool,
    /// The count the job's last START began, for the WAIT that follows.
    count: Option<mantaray_device::CountWatch>,
}

impl CliSession {
    /// A session whose relative file names resolve against `directory`,
    /// acquiring from `detector`.
    pub fn new(directory: PathBuf, trace: bool, detector: Option<AnyMcb>) -> Self {
        let mut variables = JobVariables::new();
        variables.current_dir = directory.display().to_string();
        variables.mca_dir = std::env::current_exe()
            .ok()
            .and_then(|path| path.parent().map(|parent| parent.display().to_string()))
            .unwrap_or_default();
        Self {
            variables,
            directory,
            detector,
            buffer: Spectrum::new(0),
            empty: Spectrum::new(0),
            buffer_path: None,
            target: Target::Detector,
            previous_target: Target::Detector,
            library: NuclideLibrary::new(""),
            settings: CalculationSettings::default(),
            sample_description: String::new(),
            strip_name: None,
            list_file: None,
            trace,
            count: None,
        }
    }

    fn resolve(&self, path: &str) -> PathBuf {
        let candidate = Path::new(path);
        if candidate.is_absolute() {
            candidate.to_path_buf()
        } else {
            self.directory.join(candidate)
        }
    }

    /// The instrument, or a refusal naming what is missing.
    ///
    /// A job that only reads and writes files never calls this, so it runs
    /// perfectly well with no instrument attached. One that acquires says so.
    fn instrument(&mut self) -> Result<&mut AnyMcb, JobError> {
        self.detector.as_mut().ok_or_else(|| {
            JobError::host("this job needs an instrument: give --detector host:port".to_string())
        })
    }

    /// Reads, changes and applies the presets, so they reach the instrument.
    ///
    /// `presets_mut` writes only the client-side mirror: for a remote
    /// instrument the preset would never arrive, the stop would be enforced
    /// purely from this process (and lost with it), and the no-change-while-
    /// counting rule would be bypassed. `apply_presets` is the one road every
    /// preset edit takes: it refuses a locked instrument, sends nothing for a
    /// host-side-only edit, and otherwise sends a clear and every held preset.
    fn change_presets(&mut self, change: impl FnOnce(&mut Presets)) -> Result<(), JobError> {
        let mcb = self.instrument()?;
        let mut presets = *mcb.presets();
        change(&mut presets);
        mcb.apply_presets(presets).map_err(device)
    }

    /// The spectrum commands act on.
    ///
    /// With no instrument attached the detector holds nothing; the empty
    /// spectrum here is never counts pretending to be a measurement, because
    /// every command that could acquire goes through [`Self::instrument`] and
    /// has already failed by this point.
    fn active(&self) -> &Spectrum {
        match self.target {
            Target::Buffer => &self.buffer,
            Target::Detector => match &self.detector {
                Some(detector) => detector.spectrum(),
                None => &self.empty,
            },
        }
    }

    fn active_mut(&mut self) -> &mut Spectrum {
        match self.target {
            Target::Buffer => &mut self.buffer,
            Target::Detector => match &mut self.detector {
                Some(detector) => detector.spectrum_mut(),
                None => &mut self.empty,
            },
        }
    }

    fn note(&self, message: &str) {
        if self.trace {
            eprintln!("      {message}");
        }
    }
}

fn device(error: mantaray_device::DeviceError) -> JobError {
    JobError::host(error.to_string())
}

fn io(error: impl std::fmt::Display) -> JobError {
    JobError::host(error.to_string())
}

impl JobHost for CliSession {
    fn variables(&mut self) -> &mut JobVariables {
        &mut self.variables
    }

    fn trace(&mut self, line: usize, command: &str) {
        if self.trace {
            eprintln!("{line:>4}: {command}");
        }
    }

    fn select_detector(&mut self, number: Option<u16>) -> Result<(), JobError> {
        let requested = match number {
            Some(0) => Target::Buffer,
            Some(_) => Target::Detector,
            None => self.previous_target,
        };
        self.previous_target = self.target;
        self.target = requested;
        Ok(())
    }

    fn start(&mut self) -> Result<(), JobError> {
        let instrument = self.instrument()?;
        instrument.start().map_err(device)?;
        let watch = mantaray_device::CountWatch::started(&*instrument);
        self.count = Some(watch);
        Ok(())
    }

    fn stop(&mut self) -> Result<(), JobError> {
        // The START this ends is over: a later WAIT does not watch it.
        self.count = None;
        self.instrument()?.stop().map_err(device)
    }

    fn clear(&mut self) -> Result<(), JobError> {
        match self.target {
            // The detector is not touched, so the count it was told to make
            // is still the one a later WAIT watches.
            Target::Buffer => {
                self.buffer.clear();
                Ok(())
            }
            Target::Detector => {
                // A count the job STARTed is still the one a later WAIT
                // watches (START / CLEAR / WAIT), its clocks from zero again;
                // the watch takes what the mirror holds before the CLEAR.
                if let Some(mut watch) = self.count.take() {
                    watch.clearing(&*self.instrument()?);
                    self.count = Some(watch);
                }
                self.instrument()?.clear().map_err(device)
            }
        }
    }

    fn wait_for_stop(&mut self) -> Result<(), JobError> {
        if self.instrument()?.presets().is_empty() {
            return Err(JobError::host(
                "WAIT with no preset would never finish; set a preset first",
            ));
        }
        if self.instrument()?.as_simulated().is_some() {
            // The simulator runs faster than real time: step it until a
            // preset stops it, and say so if an impossible one never does.
            let mut stepped = 0usize;
            while self.instrument()?.is_active() {
                if stepped >= 10_000_000 {
                    return Err(JobError::host(
                        "WAIT gave up: ten million simulated seconds passed and no preset stopped the acquisition",
                    ));
                }
                advance(self.instrument()?.as_mcb_mut(), 1.0).map_err(device)?;
                stepped += 1;
            }
        } else {
            // A real instrument counts in real time and stops itself when a
            // preset is reached; wait for that a few polls a second, rather
            // than hammering the wire in a tight loop. An accepted START is
            // not yet a reading that it counts (rule 9), so the count is over
            // at the first believed reading taken after this point that says
            // idle - the interface's WAIT and batch queue use the same watch.
            // Readings that stay unbelievable end the WAIT with the reason.
            // The watch the job's START began, when there is one, tells a
            // START that never counted from a count that is over.
            let mut watch = match self.count.take() {
                Some(watch) => watch,
                None => mantaray_device::CountWatch::begin(self.instrument()?),
            };
            loop {
                match watch.check(self.instrument()?) {
                    mantaray_device::CountState::Over => break,
                    mantaray_device::CountState::Counting => {}
                    mantaray_device::CountState::Unreadable(reason) => {
                        return Err(JobError::host(format!(
                            "WAIT: whether the detector counts cannot be told: {reason}"
                        )));
                    }
                    mantaray_device::CountState::NeverStarted => {
                        return Err(JobError::host(
                            "WAIT: the detector accepted START but did not count",
                        ));
                    }
                }
                let step = std::time::Duration::from_millis(250);
                std::thread::sleep(step);
                advance(self.instrument()?.as_mcb_mut(), step.as_secs_f64()).map_err(device)?;
            }
        }
        let status = self.instrument()?.status();
        self.note(&format!(
            "collected {} counts in {:.1} s live",
            status.total_counts, status.live_time
        ));
        Ok(())
    }

    fn wait_seconds(&mut self, seconds: f64) -> Result<(), JobError> {
        if !seconds.is_finite() || seconds <= 0.0 {
            return Ok(());
        }
        if self.instrument()?.as_simulated().is_some() {
            // The simulator runs faster than real time: skip straight ahead.
            if self.instrument()?.is_active() {
                advance(self.instrument()?.as_mcb_mut(), seconds).map_err(device)?;
            }
            return Ok(());
        }
        // A real instrument counts in real time, so WAIT means wall-clock
        // seconds - the manual's own START / WAIT 300 / STOP pattern used to
        // stop almost immediately and save a near-empty spectrum. Sleep in
        // steps, polling as the time passes, so the mirror stays current and
        // a host-side preset can still end the run early.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs_f64(seconds);
        loop {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            if remaining.is_zero() {
                return Ok(());
            }
            let step = remaining.min(std::time::Duration::from_millis(250));
            std::thread::sleep(step);
            advance(self.instrument()?.as_mcb_mut(), step.as_secs_f64()).map_err(device)?;
        }
    }

    fn fill_buffer(&mut self) -> Result<(), JobError> {
        self.buffer = self.instrument()?.spectrum().clone();
        self.buffer.sample_description = self.sample_description.clone();
        self.buffer_path = None;
        self.list_file = self.instrument()?.list_data().cloned();
        Ok(())
    }

    fn save(&mut self, path: Option<&str>) -> Result<(), JobError> {
        let target = match path {
            Some(path) => self.resolve(path),
            None => self
                .buffer_path
                .clone()
                .ok_or_else(|| JobError::host("SAVE without a file name and nothing recalled"))?,
        };
        let mut spectrum = self.active().clone();
        if !self.sample_description.is_empty() {
            spectrum.sample_description = self.sample_description.clone();
        }
        if SpectrumFormat::from_path(&target) == Some(SpectrumFormat::List) {
            let list = self
                .list_file
                .as_ref()
                .ok_or_else(|| JobError::host("no list-mode data to save"))?;
            let bytes = list_mode::write(list).map_err(io)?;
            mantaray_formats::write_atomic(&target, bytes).map_err(io)?;
        } else {
            save_spectrum(&spectrum, &target).map_err(io)?;
        }
        self.note(&format!("wrote {}", target.display()));
        Ok(())
    }

    fn save_rois(&mut self, path: &str) -> Result<(), JobError> {
        let target = self.resolve(path);
        save_rois(&self.active().rois, &target).map_err(io)?;
        self.note(&format!("wrote {}", target.display()));
        Ok(())
    }

    fn recall(&mut self, path: &str) -> Result<(), JobError> {
        let source = self.resolve(path);
        if SpectrumFormat::from_path(&source) == Some(SpectrumFormat::List) {
            let bytes = std::fs::read(&source).map_err(io)?;
            let list = list_mode::read(&bytes).map_err(io)?;
            self.buffer = list.to_spectrum();
            self.list_file = Some(list);
        } else {
            self.buffer = load_spectrum(&source).map_err(io)?;
            self.list_file = None;
        }
        self.buffer_path = Some(source.clone());
        self.target = Target::Buffer;
        self.variables
            .set_spectrum_path(&source.display().to_string());
        self.note(&format!(
            "recalled {} ({} channels)",
            source.display(),
            self.buffer.len()
        ));
        Ok(())
    }

    /// Recalls a calibration from a `.Clb` or from any spectrum file.
    ///
    /// The same reader and the same guards the desktop application uses, so a
    /// `.JOB` run here and the same `.JOB` run there do the same thing: a file
    /// carrying no calibration is refused rather than assigned, and a peak
    /// shape already in hand outlives a file that records none.
    fn recall_calibration(&mut self, path: &str) -> Result<(), JobError> {
        let source = self.resolve(path);
        let (energy, shape) = mantaray_formats::load_calibration(&source).map_err(io)?;
        let Some(energy) = energy else {
            return Err(JobError::host(format!(
                "{} holds no calibration",
                source.display()
            )));
        };
        let target = self.active_mut();
        target.energy_calibration = Some(energy);
        if let Some(shape) = shape {
            target.shape_calibration = Some(shape);
        }
        self.variables
            .set_spectrum_path(&source.display().to_string());
        Ok(())
    }

    fn recall_rois(&mut self, path: &str) -> Result<(), JobError> {
        let source = self.resolve(path);
        let rois = load_rois(&source).map_err(io)?;
        self.active_mut().rois = rois;
        self.variables
            .set_spectrum_path(&source.display().to_string());
        Ok(())
    }

    fn strip(&mut self, factor: f64, path: Option<&str>) -> Result<(), JobError> {
        let name = path
            .map(|value| value.to_string())
            .or_else(|| self.strip_name.clone())
            .ok_or_else(|| JobError::host("STRIP without a file name"))?;
        let source = self.resolve(&name);
        let disk = load_spectrum(&source).map_err(io)?;
        let mode = if factor == 0.0 {
            StripFactor::LiveTimeRatio
        } else {
            StripFactor::Fixed(factor)
        };
        strip(&mut self.buffer, &disk, mode).map_err(|error| JobError::host(error.to_string()))?;
        self.variables
            .set_spectrum_path(&source.display().to_string());
        Ok(())
    }

    fn set_strip_name(&mut self, path: &str) -> Result<(), JobError> {
        self.strip_name = Some(path.to_string());
        Ok(())
    }

    fn smooth(&mut self) -> Result<(), JobError> {
        smooth(&mut self.buffer);
        Ok(())
    }

    fn mark_peaks(&mut self) -> Result<(), JobError> {
        let settings = self.settings;
        let marked = mark_peaks(self.active_mut(), &settings);
        self.note(&format!("marked {marked} peak(s)"));
        Ok(())
    }

    fn report(&mut self, path: Option<&str>) -> Result<(), JobError> {
        let options = ReportOptions::paragraph()
            .with_settings(self.settings)
            .with_library(&self.library);
        let text = roi_report(self.active(), &options);
        match path {
            // "PRN" is MAESTRO's printer device; send it to standard output.
            Some(name) if name.eq_ignore_ascii_case("PRN") => print!("{text}"),
            Some(name) => {
                let target = self.resolve(name);
                mantaray_formats::write_atomic(&target, text).map_err(io)?;
                self.note(&format!("wrote {}", target.display()));
            }
            None => print!("{text}"),
        }
        Ok(())
    }

    fn describe_sample(&mut self, description: &str) -> Result<(), JobError> {
        self.sample_description = description.to_string();
        self.active_mut().sample_description = description.to_string();
        Ok(())
    }

    fn load_library(&mut self, path: &str) -> Result<(), JobError> {
        let source = self.resolve(path);
        self.library = library::load(&source).map_err(io)?;
        self.note(&format!(
            "library {} with {} nuclides",
            source.display(),
            self.library.len()
        ));
        Ok(())
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
        // Explicit: always sent, whatever the mirror or the instrument says.
        let mcb = self.instrument()?;
        mcb.set_presets(Presets::default()).map_err(device)
    }

    fn set_list_mode(&mut self, list: bool) -> Result<(), JobError> {
        let mode = if list {
            mantaray_device::AcquisitionMode::List
        } else {
            mantaray_device::AcquisitionMode::Pha
        };
        self.instrument()?.set_mode(mode).map_err(device)
    }

    fn set_list_range(&mut self, slice: &ListSlice) -> Result<(), JobError> {
        let list = self
            .list_file
            .as_ref()
            .ok_or_else(|| JobError::host("SET_RANGE without list-mode data in the buffer"))?;
        let range = match slice {
            ListSlice::Relative { start, duration } => ListRange::from_seconds(*start, *duration),
            ListSlice::Absolute {
                date,
                time,
                duration,
            } => {
                // Convert the wall-clock start into an offset from the acquisition.
                let stamp = format!("{date} {time}");
                let parsed = ["%m/%d/%Y %H:%M:%S", "%Y-%m-%d %H:%M:%S"]
                    .iter()
                    .find_map(|format| chrono::NaiveDateTime::parse_from_str(&stamp, format).ok())
                    .ok_or_else(|| JobError::host(format!("cannot read the date {stamp:?}")))?;
                let start = match list.start_time {
                    Some(begin) => (parsed - begin).num_milliseconds() as f64 / 1000.0,
                    None => 0.0,
                };
                ListRange::from_seconds(start.max(0.0), *duration)
            }
        };
        let sliced = list
            .slice(range)
            .ok_or_else(|| JobError::host("that time slice is empty"))?;
        self.buffer = sliced;
        self.target = Target::Buffer;
        Ok(())
    }

    fn send_message(&mut self, command: &str) -> Result<String, JobError> {
        // A bare START, STOP or CLEAR never arrives here: the job runs its
        // own (mantaray_jobs::exec), so the count watch follows it.
        let reply = self.instrument()?.send_message(command).map_err(device)?;
        self.note(&format!("{command} -> {reply}"));
        Ok(reply)
    }

    fn lock(&mut self, password: &str, owner: &str, _name: Option<&str>) -> Result<(), JobError> {
        self.instrument()?.lock(password, owner).map_err(device)
    }

    fn unlock(&mut self, password: &str) -> Result<(), JobError> {
        self.instrument()?.unlock(password).map_err(device)
    }

    fn beep(&mut self, spec: &BeepSpec) -> Result<(), JobError> {
        // A terminal has one sound.
        match spec {
            BeepSpec::Tone { .. } | BeepSpec::Event(_) | BeepSpec::Sound(_) => {
                eprint!("\u{7}");
                Ok(())
            }
        }
    }

    fn close_buffers(&mut self) -> Result<(), JobError> {
        self.buffer = Spectrum::new(0);
        self.buffer_path = None;
        self.target = Target::Detector;
        Ok(())
    }

    fn close_detectors(&mut self) -> Result<(), JobError> {
        self.instrument()?.stop().map_err(device)
    }

    fn export(&mut self, path: &str) -> Result<(), JobError> {
        // Without an external program configured, export writes ASCII.
        let target = self.resolve(path);
        let text = mantaray_formats::ascii::write(self.active(), &Default::default());
        mantaray_formats::write_atomic(&target, text).map_err(io)?;
        self.note(&format!("exported {}", target.display()));
        Ok(())
    }

    fn import(&mut self, path: &str) -> Result<(), JobError> {
        self.recall(path)
    }
}

/// Runs a `.JOB` file.
pub fn run_job_file(
    path: &Path,
    directory: Option<&Path>,
    trace: bool,
    detector: Option<AnyMcb>,
) -> Result<()> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading job {}", path.display()))?;
    let job = Job::parse_named(&text, &path.display().to_string())
        .map_err(|error| anyhow::anyhow!("{error}"))?;
    let directory = directory
        .map(|value| value.to_path_buf())
        .or_else(|| path.parent().map(|parent| parent.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."));
    let mut session = CliSession::new(directory, trace, detector);
    let outcome = run(&job, &mut session).map_err(|error| anyhow::anyhow!("{error}"))?;
    eprintln!(
        "job finished: {} command(s){}",
        outcome.commands,
        if outcome.quit { ", QUIT" } else { "" }
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use mantaray_device::{MockTransport, RemoteMcb};

    /// A session holding a remote instrument that answers from a script and
    /// records every command it is sent.
    ///
    /// Connecting also asks what presets the instrument already holds, so that
    /// answer is spliced in after the three-command handshake; "none set"
    /// leaves these cases as they were.
    fn remote_session(pairs: &[(&str, &str)]) -> CliSession {
        let mut script = pairs.to_vec();
        script.insert(
            pairs.len().min(3),
            ("SHOW_PRESETS", "PRESETS REAL=0.00 LIVE=0.00 PEAK=0 INTEG=0"),
        );
        let transport = MockTransport::scripted(&script);
        let remote = RemoteMcb::connect(Box::new(transport), 1, "BENCH").expect("connect");
        CliSession::new(std::env::temp_dir(), false, Some(AnyMcb::Remote(remote)))
    }

    #[test]
    fn a_job_preset_reaches_the_wire_not_just_the_mirror() {
        // The bug this pins: SET_PRESET_* wrote only the client-side mirror,
        // so the instrument never held the preset - if the process died
        // mid-job, the instrument counted forever. The scripted transport
        // refuses any command it does not expect, so passing proves the
        // presets went over the wire.
        let mut session = remote_session(&[
            (
                "SHOW_CONFIGURATION",
                "MODEL=M SERIAL=S FIRMWARE=F CHANNELS=8",
            ),
            ("SHOW_STATUS", "RT=0 LT=0 DT=0% ICR=0 ACTIVE=0 TOTAL=0"),
            ("SHOW_DATA", "DATA 2 0 0"),
            ("SHOW_STATUS", "RT=0 LT=0 DT=0% ICR=0 ACTIVE=0 TOTAL=0"),
            ("SET_PRESET_CLEAR", "OK"),
            ("SET_PRESET_LIVE 300", "OK"),
            ("SHOW_STATUS", "RT=0 LT=0 DT=0% ICR=0 ACTIVE=0 TOTAL=0"),
            ("SET_PRESET_CLEAR", "OK"),
            ("SET_PRESET_LIVE 300", "OK"),
            ("SET_PRESET_COUNT 5000", "OK"),
        ]);
        session.set_preset_live(300.0).expect("live preset");
        session.set_preset_peak(5000).expect("peak preset");
    }

    /// A session on the scripted rig instrument (direct line, as the command
    /// line holds it). Each status read moves the rig's clock half a second,
    /// so WAIT's polling sees time pass on the instrument's own terms.
    fn rig_session() -> (CliSession, mantaray_device::rig::Rig) {
        let rig = mantaray_device::rig::Rig::new(16);
        rig.tick_per_status(0.5);
        let remote = RemoteMcb::connect(rig.transport(), 1, "RIG").expect("connect");
        rig.take_sent();
        let session = CliSession::new(std::env::temp_dir(), false, Some(AnyMcb::Remote(remote)));
        (session, rig)
    }

    /// Runs `body` on its own thread and fails, rather than hangs, if it
    /// has not finished in `seconds`.
    fn within<T: Send + 'static>(
        seconds: u64,
        what: &str,
        body: impl FnOnce() -> T + Send + 'static,
    ) -> T {
        let (done, outcome) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = done.send(body());
        });
        outcome
            .recv_timeout(std::time::Duration::from_secs(seconds))
            .unwrap_or_else(|_| panic!("{what} did not finish in {seconds} s"))
    }

    #[test]
    fn rig_t16_start_then_wait_waits_for_the_instruments_own_stop() {
        let (mut session, rig) = rig_session();
        session.set_preset_live(2.0).expect("preset");
        session.start().expect("start");
        let (mut session, rig) = within(30, "WAIT", move || {
            session.wait_for_stop().map(|()| (session, rig))
        })
        .expect("wait");
        assert!(!rig.counting());
        assert!(rig.clocks().1 >= 2.0 - 1e-9, "{:?}", rig.clocks());
        let mirror = session.instrument().expect("instrument").status().live_time;
        assert!(
            mirror >= 2.0 - 0.02,
            "WAIT ended before the mirror saw the stop: {mirror}"
        );
    }

    #[test]
    fn rig_finding6_wait_on_an_idle_detector_returns_at_once() {
        let (mut session, _rig) = rig_session();
        session.set_preset_live(2.0).expect("preset");
        let began = std::time::Instant::now();
        within(30, "WAIT", move || session.wait_for_stop()).expect("wait");
        assert!(
            began.elapsed() < std::time::Duration::from_secs(3),
            "WAIT on an idle detector sat {:?}",
            began.elapsed()
        );
    }

    #[test]
    fn rig_finding2_wait_over_a_status_that_stays_unreadable_ends_with_an_error() {
        let (mut session, rig) = rig_session();
        session.set_preset_live(200.0).expect("preset");
        session.start().expect("start");
        rig.fault(mantaray_device::rig::Fault::StatusWithout {
            key: "ACTIVE".into(),
            count: usize::MAX,
        });
        let outcome = within(60, "WAIT", move || session.wait_for_stop());
        assert!(outcome.is_err(), "WAIT ended as if the count had finished");
    }

    #[test]
    fn rig_a_start_that_is_accepted_but_never_counts_fails_the_wait() {
        // The 926 can answer START and not count. A WAIT that called that
        // "over" would let the job save an empty spectrum as a measurement.
        let (mut session, rig) = rig_session();
        session.set_preset_live(2.0).expect("preset");
        rig.fault(mantaray_device::rig::Fault::Reply {
            verb: "START".into(),
            reply: "OK".into(),
            count: 1,
            execute: false,
        });
        session.start().expect("START is accepted");
        let outcome = within(30, "WAIT", move || session.wait_for_stop());
        let error = outcome.expect_err("a count that never ran is not over");
        assert!(error.to_string().contains("did not count"), "{error}");
    }

    #[test]
    fn rig_review4_a_stopped_start_does_not_haunt_a_later_wait() {
        // START, STOP before any reading saw the count, CLEAR: a later WAIT
        // is on whatever counts next, not on the START that was stopped.
        let (mut session, _rig) = rig_session();
        session.set_preset_live(3.0).expect("preset");
        session.start().expect("START");
        session.stop().expect("STOP");
        session.clear().expect("CLEAR");
        let outcome = within(30, "WAIT", move || session.wait_for_stop());
        outcome.expect("WAIT on an idle detector is over at once");
    }

    #[test]
    fn rig_review_r2_a_raw_stop_and_clear_drop_the_watch() {
        // Through the job runner: a bare raw STOP or CLEAR is the job's own.
        let (mut session, _rig) = rig_session();
        let job = Job::parse(
            "SET_PRESET_LIVE 3\nSTART\nSEND_MESSAGE \"STOP\"\nSEND_MESSAGE \"CLEAR\"\nWAIT\n",
        )
        .expect("job");
        let outcome = within(30, "the job", move || run(&job, &mut session).map(|_| ()));
        outcome.expect("WAIT on an idle detector is over at once");
    }

    #[test]
    fn rig_review_r2_a_raw_start_is_watched_like_a_typed_one() {
        // A raw START the detector accepted but did not count is reported,
        // not taken for a count that is over.
        let (mut session, rig) = rig_session();
        rig.fault(mantaray_device::rig::Fault::Reply {
            verb: "START".into(),
            reply: "OK".into(),
            count: 1,
            execute: false,
        });
        let job = Job::parse("SET_PRESET_LIVE 2\nSEND_MESSAGE \"START\"\nWAIT\n").expect("job");
        let outcome = within(30, "the job", move || run(&job, &mut session).map(|_| ()));
        let error = outcome.expect_err("a count that never ran is not over");
        assert!(error.to_string().contains("did not count"), "{error}");
    }

    #[test]
    fn rig_a_job_preset_on_a_front_panel_count_is_refused() {
        let (mut session, rig) = rig_session();
        rig.front_panel_start();
        let error = session.set_preset_live(5.0).expect_err("counting");
        assert!(
            error
                .to_string()
                .contains("Presets can't be changed during acquisition."),
            "{error}"
        );
        assert!(rig.violations().is_empty(), "{:?}", rig.violations());
    }

    #[test]
    fn a_job_that_sets_a_preset_mid_count_stops_with_maestros_message() {
        // MAESTRO: "Presets can't be changed during acquisition." The
        // scripted transport refuses any command it does not expect and the
        // instrument is counting, so nothing may go out; a host-side preset
        // is refused as well, as the whole preset change is.
        let mut session = remote_session(&[
            (
                "SHOW_CONFIGURATION",
                "MODEL=M SERIAL=S FIRMWARE=F CHANNELS=8",
            ),
            ("SHOW_STATUS", "RT=5 LT=5 DT=0% ICR=0 ACTIVE=1 TOTAL=0"),
            ("SHOW_DATA", "DATA 2 0 0"),
            // One status read per refused change, and nothing else.
            ("SHOW_STATUS", "RT=5 LT=5 DT=0% ICR=0 ACTIVE=1 TOTAL=0"),
            ("SHOW_STATUS", "RT=5 LT=5 DT=0% ICR=0 ACTIVE=1 TOTAL=0"),
            ("SHOW_STATUS", "RT=5 LT=5 DT=0% ICR=0 ACTIVE=1 TOTAL=0"),
        ]);
        let words = "Presets can't be changed during acquisition.";
        let live = session.set_preset_live(300.0).expect_err("live mid-count");
        assert!(live.to_string().contains(words), "{live}");
        let host = session
            .set_preset_uncertainty(2.0, 0, 1)
            .expect_err("host-side mid-count");
        assert!(host.to_string().contains(words), "{host}");
        let clear = session.clear_presets().expect_err("clear mid-count");
        assert!(clear.to_string().contains(words), "{clear}");
        let held = *session.instrument().expect("instrument").presets();
        assert!(held.uncertainty.is_none(), "the mirror is untouched");
    }

    #[test]
    fn a_locked_instrument_refuses_a_job_s_host_side_preset() {
        let mut session = remote_session(&[
            (
                "SHOW_CONFIGURATION",
                "MODEL=M SERIAL=S FIRMWARE=F CHANNELS=8",
            ),
            ("SHOW_STATUS", "RT=0 LT=0 DT=0% ICR=0 ACTIVE=0 TOTAL=0"),
            ("SHOW_DATA", "DATA 2 0 0"),
        ]);
        session
            .instrument()
            .expect("instrument")
            .lock("pw", "someone")
            .expect("lock");
        session
            .set_preset_uncertainty(2.0, 0, 1)
            .expect_err("locked");
        assert!(
            session
                .instrument()
                .expect("instrument")
                .presets()
                .uncertainty
                .is_none()
        );
    }

    #[test]
    fn wait_on_a_real_instrument_passes_real_time() {
        // The manual's own START / WAIT 300 / STOP pattern used to stop
        // almost immediately: advance() on a remote instrument only bumps a
        // poll counter, it does not wait. WAIT must spend the seconds it
        // names. The script carries the one refresh the polling crosses.
        let mut session = remote_session(&[
            (
                "SHOW_CONFIGURATION",
                "MODEL=M SERIAL=S FIRMWARE=F CHANNELS=8",
            ),
            ("SHOW_STATUS", "RT=0 LT=0 DT=0% ICR=0 ACTIVE=1 TOTAL=0"),
            ("SHOW_DATA", "DATA 2 0 0"),
            ("SHOW_STATUS", "RT=1 LT=1 DT=0% ICR=0 ACTIVE=1 TOTAL=5"),
            ("SHOW_DATA", "DATA 2 2 3"),
        ]);
        let before = std::time::Instant::now();
        session.wait_seconds(0.6).expect("wait");
        assert!(
            before.elapsed() >= std::time::Duration::from_millis(600),
            "WAIT 0.6 returned after {:?}",
            before.elapsed()
        );
    }

    #[test]
    fn start_then_wait_waits_for_the_detector_to_stop_itself() {
        // T16 (rule 9 as amended): START is decided by its reply, with no
        // reading after it, so the WAIT after it waits for readings: one
        // that says stopped with the clock run - not for a flag the program
        // wrote itself. The script refuses anything else.
        let mut session = remote_session(&[
            (
                "SHOW_CONFIGURATION",
                "MODEL=M SERIAL=S FIRMWARE=F CHANNELS=8",
            ),
            ("SHOW_STATUS", "RT=0 LT=0 DT=0% ICR=0 ACTIVE=0 TOTAL=0"),
            ("SHOW_DATA", "DATA 2 0 0"),
            // SET_PRESET_LIVE 10: the gate's read, the clear, the preset.
            ("SHOW_STATUS", "RT=0 LT=0 DT=0% ICR=0 ACTIVE=0 TOTAL=0"),
            ("SET_PRESET_CLEAR", "OK"),
            ("SET_PRESET_LIVE 10", "OK"),
            // START, after the read that says no held preset is reached.
            ("SHOW_STATUS", "RT=0 LT=0 DT=0% ICR=0 ACTIVE=0 TOTAL=0"),
            ("START", "OK"),
            // WAIT: still counting, then stopped on its own preset.
            ("SHOW_STATUS", "RT=5 LT=5 DT=0% ICR=0 ACTIVE=1 TOTAL=3"),
            ("SHOW_DATA", "DATA 2 1 2"),
            ("SHOW_STATUS", "RT=10 LT=10 DT=0% ICR=0 ACTIVE=0 TOTAL=7"),
            ("SHOW_DATA", "DATA 2 3 4"),
        ]);
        session.set_preset_live(10.0).expect("preset");
        session.start().expect("start");
        assert!(
            !session.instrument().expect("instrument").is_active(),
            "no reading has said it counts yet"
        );
        session.wait_for_stop().expect("wait");
        let instrument = session.instrument().expect("instrument");
        assert!(!instrument.is_active());
        assert_eq!(instrument.status().total_counts, 7);
        assert_eq!(instrument.spectrum().total_counts(), 7);
    }
}
