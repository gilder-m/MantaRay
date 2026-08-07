//! The `.JOB` parser.

use crate::command::{BeepSpec, Command, ListSlice};
use crate::error::JobError;

/// One parsed line.
#[derive(Clone, Debug, PartialEq)]
pub struct JobLine {
    /// One-based line number in the file.
    pub line: usize,
    /// The command on it.
    pub command: Command,
}

/// A parsed `.JOB` file.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Job {
    /// Where the job came from, for messages.
    pub name: String,
    commands: Vec<JobLine>,
}

impl Job {
    /// Parses job text.
    pub fn parse(text: &str) -> Result<Self, JobError> {
        let mut commands = Vec::new();
        for (index, raw) in text.lines().enumerate() {
            let line = index + 1;
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                continue;
            }
            commands.push(JobLine {
                line,
                command: parse_line(trimmed, line)?,
            });
        }
        Ok(Self {
            name: String::new(),
            commands,
        })
    }

    /// Parses job text, recording where it came from.
    pub fn parse_named(text: &str, name: &str) -> Result<Self, JobError> {
        let mut job = Self::parse(text)?;
        job.name = name.to_string();
        Ok(job)
    }

    /// The parsed commands.
    pub fn commands(&self) -> &[JobLine] {
        &self.commands
    }

    /// Number of commands.
    pub fn len(&self) -> usize {
        self.commands.len()
    }

    /// True when the job has no commands.
    pub fn is_empty(&self) -> bool {
        self.commands.is_empty()
    }

    /// How many loops the job contains.
    pub fn loop_count(&self) -> usize {
        self.commands
            .iter()
            .filter(|entry| matches!(entry.command, Command::Loop(_) | Command::LoopSpectra))
            .count()
    }
}

fn parse_line(text: &str, line: usize) -> Result<Command, JobError> {
    let (word, rest) = split_word(text);
    let upper = word.to_ascii_uppercase();

    // REM keeps its text verbatim.
    if upper == "REM" {
        return Ok(Command::Rem(rest.to_string()));
    }

    let args = split_args(rest);
    let bad = |detail: String| JobError::BadArgument {
        line,
        command: upper.clone(),
        detail,
    };
    let text_arg = |index: usize| -> Result<String, JobError> {
        args.get(index)
            .cloned()
            .ok_or_else(|| bad(format!("argument {} is missing", index + 1)))
    };
    let number = |index: usize| -> Result<f64, JobError> {
        let value = text_arg(index)?;
        value
            .parse::<f64>()
            .map_err(|_| bad(format!("{value:?} is not a number")))
    };

    let command = match upper.as_str() {
        "ASK_PASSWORD" => Command::AskPassword,
        "BEEP" => match args.len() {
            0 => return Err(bad("BEEP needs an argument".into())),
            1 => match args[0].parse::<u32>() {
                Ok(id) => Command::Beep(BeepSpec::Event(id)),
                Err(_) => Command::Beep(BeepSpec::Sound(args[0].clone())),
            },
            _ => Command::Beep(BeepSpec::Tone {
                frequency: number(0)? as u32,
                duration: number(1)? as u32,
            }),
        },
        "CHANGE_SAMPLE" => Command::ChangeSample,
        "CLEAR" => Command::Clear,
        "CLOSEBUFFERS" | "CLOSE_BUFFERS" => Command::CloseBuffers,
        "CLOSEMCBS" | "CLOSE_MCBS" => Command::CloseMcbs,
        "DESCRIBE_SAMPLE" => Command::DescribeSample(text_arg(0)?),
        "EXPORT" => Command::Export(text_arg(0)?),
        "FILL_BUFFER" => Command::FillBuffer,
        "IMPORT" => Command::Import(text_arg(0)?),
        "LOAD_LIBRARY" => Command::LoadLibrary(text_arg(0)?),
        "LOCK" => Command::Lock {
            password: text_arg(0)?,
            owner: args.get(1).cloned().unwrap_or_default(),
            name: args.get(2).cloned(),
        },
        "LOOP" => match args.first().map(|value| value.to_ascii_uppercase()) {
            Some(word) if word == "SPECTRA" => Command::LoopSpectra,
            Some(_) => Command::Loop(number(0)?.max(0.0) as u32),
            None => return Err(bad("LOOP needs a repetition count".into())),
        },
        "END_LOOP" | "ENDLOOP" => Command::EndLoop,
        "MARK_PEAKS" => Command::MarkPeaks,
        "QUIT" => Command::Quit,
        "RECALL" => Command::Recall(text_arg(0)?),
        "RECALL_CALIB" | "RECALL_CALIBRATION" => Command::RecallCalibration(text_arg(0)?),
        "RECALL_ROI" => Command::RecallRoi(text_arg(0)?),
        "REPORT" => Command::Report(args.first().cloned()),
        "RUN" => Command::Run {
            program: text_arg(0)?,
            minimized: false,
        },
        "RUN_MINIMIZED" => Command::Run {
            program: text_arg(0)?,
            minimized: true,
        },
        "SAVE" => Command::Save(args.first().cloned()),
        "SAVE_ROI" => Command::SaveRoi(text_arg(0)?),
        "SEND_MESSAGE" => Command::SendMessage(text_arg(0)?),
        "SET_BUFFER" => Command::SetBuffer,
        "SET_DETECTOR" => match args.first() {
            Some(_) => Command::SetDetector(Some(number(0)?.max(0.0) as u16)),
            None => Command::SetDetector(None),
        },
        "SET_LIST" => Command::SetList,
        "SET_NAME_STRIP" => Command::SetNameStrip(text_arg(0)?),
        "SET_PHA" => Command::SetPha,
        "SET_PRESET_CLEAR" => Command::SetPresetClear,
        "SET_PRESET_COUNT" => Command::SetPresetCount(number(0)?.max(0.0) as u64),
        "SET_PRESET_INTEG" | "SET_PRESET_INTEGRAL" => {
            Command::SetPresetIntegral(number(0)?.max(0.0) as u64)
        }
        "SET_PRESET_LIVE" => Command::SetPresetLive(number(0)?),
        "SET_PRESET_REAL" => Command::SetPresetReal(number(0)?),
        "SET_PRESET_UNCERTAINTY" => Command::SetPresetUncertainty {
            limit: number(0)?,
            low: number(1)?.max(0.0) as usize,
            high: number(2)?.max(0.0) as usize,
        },
        "SET_RANGE" => match args.len() {
            3 => Command::SetRange(ListSlice::Absolute {
                date: text_arg(0)?,
                time: text_arg(1)?,
                duration: number(2)?,
            }),
            2 => Command::SetRange(ListSlice::Relative {
                start: number(0)?,
                duration: number(1)?,
            }),
            _ => return Err(bad("SET_RANGE takes two or three arguments".into())),
        },
        "SMOOTH" => Command::Smooth,
        "START" => Command::Start,
        "START_OPTIMIZE" => Command::StartOptimize,
        "START_PZ" => Command::StartPz,
        "STOP" => Command::Stop,
        "STOP_PZ" => Command::StopPz,
        "STRIP" => Command::Strip {
            factor: number(0)?,
            file: args.get(1).cloned(),
        },
        "UNLOCK" => Command::Unlock(text_arg(0)?),
        "VIEW" => Command::View(number(0)?.max(0.0) as usize),
        "WAIT" => match args.first() {
            None => Command::Wait(None),
            Some(value) => match value.parse::<f64>() {
                Ok(seconds) => Command::Wait(Some(seconds)),
                Err(_) => Command::WaitProgram(value.clone()),
            },
        },
        "WAIT_AUTO" => Command::WaitAuto,
        "WAIT_CHANGER" => Command::WaitChanger,
        "WAIT_PZ" => Command::WaitPz,
        "ZOOM" => Command::Zoom(number(0)? as i32),
        "ZOOM:" => Command::ZoomRect {
            x: number(0)? as i32,
            y: number(1)? as i32,
            width: number(2)? as i32,
            height: number(3)? as i32,
        },
        other => {
            return Err(JobError::UnknownCommand {
                line,
                word: other.to_string(),
            });
        }
    };
    Ok(command)
}

/// Splits the command word from the rest of the line.
fn split_word(text: &str) -> (&str, &str) {
    match text.find(char::is_whitespace) {
        Some(index) => (&text[..index], text[index..].trim_start()),
        None => (text, ""),
    }
}

/// Splits arguments, honouring double quotes and treating commas and whitespace
/// as separators.
fn split_args(text: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut pushed_quoted = false;
    for character in text.chars() {
        match character {
            '"' => {
                if in_quotes {
                    args.push(std::mem::take(&mut current));
                    pushed_quoted = true;
                }
                in_quotes = !in_quotes;
            }
            ',' if !in_quotes => {
                if !current.trim().is_empty() {
                    args.push(current.trim().to_string());
                }
                current.clear();
                pushed_quoted = false;
            }
            c if c.is_whitespace() && !in_quotes => {
                if !current.trim().is_empty() {
                    args.push(current.trim().to_string());
                    current.clear();
                }
                // Whitespace ends an argument, quoted or not: without this,
                // everything after `"pw" ` glues onto the previous argument.
                pushed_quoted = false;
            }
            c => {
                if pushed_quoted {
                    // Text directly after a closing quote continues that argument.
                    if let Some(last) = args.last_mut() {
                        last.push(c);
                    }
                } else {
                    current.push(c);
                }
            }
        }
    }
    if !current.trim().is_empty() {
        args.push(current.trim().to_string());
    }
    args
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoted_arguments_keep_their_spaces() {
        assert_eq!(
            split_args("\"SET_GAIN_FINE 2048\""),
            vec!["SET_GAIN_FINE 2048"]
        );
    }

    #[test]
    fn commas_and_spaces_both_separate() {
        assert_eq!(split_args("1000,1000"), vec!["1000", "1000"]);
        assert_eq!(split_args("0,\"back.chn\""), vec!["0", "back.chn"]);
        assert_eq!(split_args("10, 20 , 30"), vec!["10", "20", "30"]);
    }

    #[test]
    fn an_unquoted_name_is_one_argument() {
        assert_eq!(split_args("TEST001.CHN"), vec!["TEST001.CHN"]);
    }

    #[test]
    fn space_separated_quoted_arguments_stay_separate() {
        // The bug this pins: only a comma reset the after-quote state, so
        // `LOCK "pw" "owner"` locked the detector with password "pwowner"
        // and owner "" - a password the operator never typed.
        assert_eq!(split_args("\"pw\" \"owner\""), vec!["pw", "owner"]);
        assert_eq!(
            split_args("\"6/29/2012\" \"14:05:00\" 900"),
            vec!["6/29/2012", "14:05:00", "900"]
        );
    }

    #[test]
    fn text_directly_after_a_closing_quote_still_continues_the_argument() {
        assert_eq!(split_args("\"a b\"c"), vec!["a bc"]);
        assert_eq!(split_args("\"a\"bc \"d\""), vec!["abc", "d"]);
    }

    #[test]
    fn the_word_splitter_handles_a_bare_command() {
        assert_eq!(split_word("START"), ("START", ""));
        assert_eq!(split_word("SAVE  \"a.chn\""), ("SAVE", "\"a.chn\""));
    }
}
