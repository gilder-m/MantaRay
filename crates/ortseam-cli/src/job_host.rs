//! A headless [`JobHost`]: runs `.JOB` files against an instrument and the file
//! system.
//!
//! The instrument is whatever the caller hands over - a network MCB on the
//! bench, or the simulator in a build that has one. A job that acquires needs
//! something to acquire from; there is no fallback that invents counts.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use ortseam_core::{
    CalculationSettings, NuclideLibrary, Spectrum, StripFactor, mark_peaks, smooth, strip,
};
use ortseam_device::{AnyMcb, Mcb, Presets, UncertaintyPreset, advance};
use ortseam_formats::{
    ListRange, SpectrumFormat, library, list_mode, load_rois, load_spectrum, save_rois,
    save_spectrum,
};
use ortseam_jobs::{BeepSpec, Job, JobError, JobHost, JobVariables, ListSlice, run};
use ortseam_report::{ReportOptions, roi_report};

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
    list_file: Option<ortseam_formats::ListModeFile>,
    trace: bool,
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
            library: NuclideLibrary::standard(),
            settings: CalculationSettings::default(),
            sample_description: String::new(),
            strip_name: None,
            list_file: None,
            trace,
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
    /// counting rule would be bypassed. `set_presets` is the path that
    /// transmits.
    fn change_presets(&mut self, change: impl FnOnce(&mut Presets)) -> Result<(), JobError> {
        let mcb = self.instrument()?;
        let mut presets = *mcb.presets();
        change(&mut presets);
        mcb.set_presets(presets).map_err(device)
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

fn device(error: ortseam_device::DeviceError) -> JobError {
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
        self.instrument()?.start().map_err(device)
    }

    fn stop(&mut self) -> Result<(), JobError> {
        self.instrument()?.stop().map_err(device)
    }

    fn clear(&mut self) -> Result<(), JobError> {
        match self.target {
            Target::Buffer => {
                self.buffer.clear();
                Ok(())
            }
            Target::Detector => self.instrument()?.clear().map_err(device),
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
            // than hammering the wire in a tight loop.
            while self.instrument()?.is_active() {
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
            std::fs::write(&target, bytes).map_err(io)?;
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

    fn recall_calibration(&mut self, path: &str) -> Result<(), JobError> {
        let source = self.resolve(path);
        let other = load_spectrum(&source).map_err(io)?;
        let (energy, shape) = (other.energy_calibration, other.shape_calibration);
        let target = self.active_mut();
        target.energy_calibration = energy;
        target.shape_calibration = shape;
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
                std::fs::write(&target, text).map_err(io)?;
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
        self.change_presets(|presets| presets.real_time = Some(seconds))
    }

    fn set_preset_live(&mut self, seconds: f64) -> Result<(), JobError> {
        self.change_presets(|presets| presets.live_time = Some(seconds))
    }

    fn set_preset_peak(&mut self, counts: u64) -> Result<(), JobError> {
        self.change_presets(|presets| presets.roi_peak = Some(counts))
    }

    fn set_preset_integral(&mut self, counts: u64) -> Result<(), JobError> {
        self.change_presets(|presets| presets.roi_integral = Some(counts))
    }

    fn set_preset_uncertainty(
        &mut self,
        limit: f64,
        low: usize,
        high: usize,
    ) -> Result<(), JobError> {
        self.change_presets(|presets| {
            presets.uncertainty = Some(UncertaintyPreset {
                limit_percent: limit,
                low_channel: low,
                high_channel: high,
            })
        })
    }

    fn clear_presets(&mut self) -> Result<(), JobError> {
        self.change_presets(Presets::clear)
    }

    fn set_list_mode(&mut self, list: bool) -> Result<(), JobError> {
        let mode = if list {
            ortseam_device::AcquisitionMode::List
        } else {
            ortseam_device::AcquisitionMode::Pha
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
        let text = ortseam_formats::ascii::write(self.active(), &Default::default());
        std::fs::write(&target, text).map_err(io)?;
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
    use ortseam_device::{MockTransport, RemoteMcb};

    /// A session holding a remote instrument that answers from a script and
    /// records every command it is sent.
    fn remote_session(pairs: &[(&str, &str)]) -> CliSession {
        let transport = MockTransport::scripted(pairs);
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
            ("SET_PRESET_CLEAR", "OK"),
            ("SET_PRESET_LIVE 300", "OK"),
            ("SET_PRESET_CLEAR", "OK"),
            ("SET_PRESET_LIVE 300", "OK"),
            ("SET_PRESET_COUNT 5000", "OK"),
        ]);
        session.set_preset_live(300.0).expect("live preset");
        session.set_preset_peak(5000).expect("peak preset");
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
}
