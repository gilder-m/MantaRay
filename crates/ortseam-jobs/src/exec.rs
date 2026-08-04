//! Running a parsed job.
//!
//! [`run`] carries a whole job out at once. [`Runner`] does it one command at a
//! time, so an interactive program can keep drawing between commands.

use crate::command::Command;
use crate::error::JobError;
use crate::host::JobHost;
use crate::parse::{Job, JobLine};

/// What a completed run did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct JobOutcome {
    /// How many commands were carried out.
    pub commands: usize,
    /// Whether the job asked the program to exit.
    pub quit: bool,
}

/// State of the loop currently being executed.
#[derive(Clone, Copy, Debug)]
struct LoopState {
    /// First command of the body.
    start: usize,
    /// Index of the `END_LOOP`, or the end of the job.
    end: usize,
    /// Pass being executed, zero based.
    pass: u32,
    /// How many passes to make.
    passes: u32,
}

/// Runs a job one command at a time.
pub struct Runner {
    job: Job,
    index: usize,
    loop_state: Option<LoopState>,
    outcome: JobOutcome,
    finished: bool,
}

impl Runner {
    /// Prepares a job to be stepped.
    pub fn new(job: Job) -> Self {
        Self {
            job,
            index: 0,
            loop_state: None,
            outcome: JobOutcome::default(),
            finished: false,
        }
    }

    /// The job being run.
    pub fn job(&self) -> &Job {
        &self.job
    }

    /// Number of commands in the job.
    pub fn len(&self) -> usize {
        self.job.len()
    }

    /// True when the job has no commands.
    pub fn is_empty(&self) -> bool {
        self.job.is_empty()
    }

    /// Line number of the next command, one based.
    pub fn position(&self) -> usize {
        self.job
            .commands()
            .get(self.index)
            .map(|entry| entry.line)
            .unwrap_or_else(|| self.job.len())
    }

    /// Which pass of the current loop is running, if any.
    pub fn loop_pass(&self) -> Option<(u32, u32)> {
        self.loop_state.map(|state| (state.pass + 1, state.passes))
    }

    /// Whether the job has finished.
    pub fn is_finished(&self) -> bool {
        self.finished
    }

    /// What the run did so far.
    pub fn outcome(&self) -> JobOutcome {
        self.outcome
    }

    /// Carries out the next command. Returns whether more remain.
    pub fn step(&mut self, host: &mut dyn JobHost) -> Result<bool, JobError> {
        if self.finished {
            return Ok(false);
        }
        let entries = self.job.commands();

        // End of the job, or of a loop body.
        if let Some(mut state) = self.loop_state
            && self.index >= state.end
        {
            state.pass += 1;
            if state.pass < state.passes {
                state.pass_start(&mut self.index);
                self.loop_state = Some(state);
                host.variables().set_loop(state.pass);
                return Ok(true);
            }
            self.loop_state = None;
            host.variables().set_loop(0);
            self.index = if state.end < entries.len() {
                state.end + 1
            } else {
                entries.len()
            };
        }
        if self.index >= entries.len() {
            self.finished = true;
            return Ok(false);
        }

        let entry = &entries[self.index];
        match &entry.command {
            Command::Loop(repetitions) => {
                if self.loop_state.is_some() {
                    return Err(JobError::NestedLoop { line: entry.line });
                }
                let (end, closed) = find_end(entries, self.index + 1)?;
                let passes = if closed { (*repetitions).max(1) } else { 1 };
                self.loop_state = Some(LoopState {
                    start: self.index + 1,
                    end,
                    pass: 0,
                    passes,
                });
                host.variables().set_loop(0);
                self.index += 1;
            }
            Command::LoopSpectra => {
                if self.loop_state.is_some() {
                    return Err(JobError::NestedLoop { line: entry.line });
                }
                let (end, closed) = find_end(entries, self.index + 1)?;
                let passes = if closed {
                    host.stored_spectra()
                        .map_err(|error| error.at_line(entry.line))?
                        .max(1) as u32
                } else {
                    1
                };
                self.loop_state = Some(LoopState {
                    start: self.index + 1,
                    end,
                    pass: 0,
                    passes,
                });
                host.variables().set_loop(0);
                self.index += 1;
            }
            Command::EndLoop => {
                // Handled by the loop bookkeeping above.
                self.index += 1;
            }
            _ => {
                let entry = entry.clone();
                execute(&entry, host, &mut self.outcome)?;
                self.index += 1;
                if self.outcome.quit {
                    self.finished = true;
                    return Ok(false);
                }
            }
        }
        Ok(true)
    }
}

impl LoopState {
    fn pass_start(&self, index: &mut usize) {
        *index = self.start;
    }
}

/// Runs a whole job against a host.
pub fn run(job: &Job, host: &mut dyn JobHost) -> Result<JobOutcome, JobError> {
    let mut runner = Runner::new(job.clone());
    // A generous bound: enough for any real job, small enough to catch a runaway.
    for _ in 0..1_000_000 {
        if !runner.step(host)? {
            return Ok(runner.outcome());
        }
    }
    Err(JobError::Aborted)
}

/// Finds the `END_LOOP` that closes a loop, rejecting a nested one.
///
/// Returns the index of the terminator and whether one was actually there; an
/// unterminated loop runs its body to the end of the file exactly once.
fn find_end(entries: &[JobLine], from: usize) -> Result<(usize, bool), JobError> {
    for (offset, entry) in entries[from..].iter().enumerate() {
        match entry.command {
            Command::Loop(_) | Command::LoopSpectra => {
                return Err(JobError::NestedLoop { line: entry.line });
            }
            Command::EndLoop => return Ok((from + offset, true)),
            _ => {}
        }
    }
    Ok((entries.len(), false))
}

fn execute(
    entry: &JobLine,
    host: &mut dyn JobHost,
    outcome: &mut JobOutcome,
) -> Result<(), JobError> {
    let line = entry.line;
    host.trace(line, entry.command.word());
    outcome.commands += 1;

    macro_rules! expand {
        ($text:expr) => {
            host.variables().expand($text)
        };
    }

    let result = match &entry.command {
        Command::Rem(_) => Ok(()),
        Command::AskPassword => match host.ask_password() {
            Ok((password, owner)) => {
                let variables = host.variables();
                variables.password = password;
                variables.owner = owner;
                Ok(())
            }
            Err(error) => Err(error),
        },
        Command::Beep(spec) => host.beep(spec),
        Command::ChangeSample => host.change_sample(),
        Command::Clear => host.clear(),
        Command::CloseBuffers => host.close_buffers(),
        Command::CloseMcbs => host.close_detectors(),
        Command::DescribeSample(text) => {
            let text = expand!(text);
            host.describe_sample(&text)
        }
        Command::Export(path) => {
            let path = expand!(path);
            host.export(&path)
        }
        Command::FillBuffer => host.fill_buffer(),
        Command::Import(path) => {
            let path = expand!(path);
            host.import(&path)
        }
        Command::LoadLibrary(path) => {
            let path = expand!(path);
            host.load_library(&path)
        }
        Command::Lock {
            password,
            owner,
            name,
        } => {
            let password = expand!(password);
            let owner = expand!(owner);
            let name = name.as_ref().map(|value| expand!(value));
            host.lock(&password, &owner, name.as_deref())
        }
        Command::Loop(_) | Command::LoopSpectra | Command::EndLoop => Ok(()),
        Command::MarkPeaks => host.mark_peaks(),
        Command::Quit => {
            outcome.quit = true;
            host.quit()
        }
        Command::Recall(path) => {
            let path = expand!(path);
            host.recall(&path)
        }
        Command::RecallCalibration(path) => {
            let path = expand!(path);
            host.recall_calibration(&path)
        }
        Command::RecallRoi(path) => {
            let path = expand!(path);
            host.recall_rois(&path)
        }
        Command::Report(path) => match path {
            Some(path) => {
                let path = expand!(path);
                host.report(Some(&path))
            }
            None => host.report(None),
        },
        Command::Run { program, minimized } => {
            let program = expand!(program);
            host.run_program(&program, *minimized)
        }
        Command::Save(path) => match path {
            Some(path) => {
                let path = expand!(path);
                host.save(Some(&path))
            }
            None => host.save(None),
        },
        Command::SaveRoi(path) => {
            let path = expand!(path);
            host.save_rois(&path)
        }
        Command::SendMessage(message) => {
            let message = expand!(message);
            host.send_message(&message).map(|_| ())
        }
        Command::SetBuffer => host.select_detector(Some(0)),
        Command::SetDetector(number) => host.select_detector(*number),
        Command::SetList => host.set_list_mode(true),
        Command::SetNameStrip(path) => {
            let path = expand!(path);
            host.set_strip_name(&path)
        }
        Command::SetPha => host.set_list_mode(false),
        Command::SetPresetClear => host.clear_presets(),
        Command::SetPresetCount(counts) => host.set_preset_peak(*counts),
        Command::SetPresetIntegral(counts) => host.set_preset_integral(*counts),
        Command::SetPresetLive(seconds) => host.set_preset_live(*seconds),
        Command::SetPresetReal(seconds) => host.set_preset_real(*seconds),
        Command::SetPresetUncertainty { limit, low, high } => {
            host.set_preset_uncertainty(*limit, *low, *high)
        }
        Command::SetRange(slice) => host.set_list_range(slice),
        Command::Smooth => host.smooth(),
        Command::Start => host.start(),
        Command::StartOptimize => host.start_optimize(),
        Command::StartPz => host.pole_zero(true),
        Command::Stop => host.stop(),
        Command::StopPz => host.pole_zero(false),
        Command::Strip { factor, file } => match file {
            Some(path) => {
                let path = expand!(path);
                host.strip(*factor, Some(&path))
            }
            None => host.strip(*factor, None),
        },
        Command::Unlock(password) => {
            let password = expand!(password);
            host.unlock(&password)
        }
        Command::View(index) => host.view_stored(*index),
        Command::Wait(seconds) => match seconds {
            Some(seconds) => host.wait_seconds(*seconds),
            None => host.wait_for_stop(),
        },
        Command::WaitAuto => host.wait_optimize(),
        Command::WaitChanger => host.wait_changer(),
        Command::WaitProgram(program) => {
            let program = expand!(program);
            host.wait_program(&program)
        }
        Command::WaitPz => host.wait_pole_zero(),
        Command::Zoom(state) => host.zoom(None, *state),
        Command::ZoomRect {
            x,
            y,
            width,
            height,
        } => host.zoom(Some((*x, *y, *width, *height)), 1),
    };
    result.map_err(|error| error.at_line(line))
}
