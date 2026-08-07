//! Running a `.JOB`: loops, variables and the host interface.

use mantaray_jobs::{Job, JobError, JobHost, JobVariables, run};

/// A host that records what a job asked it to do.
#[derive(Default)]
struct Recorder {
    log: Vec<String>,
    variables: JobVariables,
    stop_after: Option<usize>,
}

impl Recorder {
    fn note(&mut self, text: impl Into<String>) -> Result<(), JobError> {
        let text = text.into();
        self.log.push(text);
        match self.stop_after {
            Some(limit) if self.log.len() > limit => Err(JobError::Aborted),
            _ => Ok(()),
        }
    }
}

impl JobHost for Recorder {
    fn variables(&mut self) -> &mut JobVariables {
        &mut self.variables
    }

    fn select_detector(&mut self, number: Option<u16>) -> Result<(), JobError> {
        self.note(format!("detector {number:?}"))
    }

    fn start(&mut self) -> Result<(), JobError> {
        self.note("start")
    }

    fn stop(&mut self) -> Result<(), JobError> {
        self.note("stop")
    }

    fn clear(&mut self) -> Result<(), JobError> {
        self.note("clear")
    }

    fn wait_for_stop(&mut self) -> Result<(), JobError> {
        self.note("wait")
    }

    fn wait_seconds(&mut self, seconds: f64) -> Result<(), JobError> {
        self.note(format!("wait {seconds}"))
    }

    fn fill_buffer(&mut self) -> Result<(), JobError> {
        self.note("fill_buffer")
    }

    fn save(&mut self, path: Option<&str>) -> Result<(), JobError> {
        self.note(format!("save {}", path.unwrap_or("<current>")))
    }

    fn recall(&mut self, path: &str) -> Result<(), JobError> {
        self.variables.set_spectrum_path(path);
        self.note(format!("recall {path}"))
    }

    fn describe_sample(&mut self, text: &str) -> Result<(), JobError> {
        self.note(format!("describe {text}"))
    }

    fn report(&mut self, path: Option<&str>) -> Result<(), JobError> {
        self.note(format!("report {}", path.unwrap_or("<printer>")))
    }

    fn set_preset_live(&mut self, seconds: f64) -> Result<(), JobError> {
        self.note(format!("preset live {seconds}"))
    }

    fn clear_presets(&mut self) -> Result<(), JobError> {
        self.note("preset clear")
    }
}

fn run_text(text: &str) -> Vec<String> {
    let job = Job::parse(text).expect("parse");
    let mut host = Recorder::default();
    run(&job, &mut host).expect("run");
    host.log
}

#[test]
fn commands_run_in_order() {
    let log = run_text("SET_DETECTOR 1\nCLEAR\nSTART\nWAIT\nSTOP\n");
    assert_eq!(
        log,
        vec!["detector Some(1)", "clear", "start", "wait", "stop"]
    );
}

#[test]
fn comments_do_nothing() {
    assert_eq!(run_text("REM nothing here\nSTART\n"), vec!["start"]);
}

#[test]
fn a_loop_repeats_its_body() {
    let log = run_text("LOOP 3\nCLEAR\nSTART\nEND_LOOP\n");
    assert_eq!(log.len(), 6);
    assert_eq!(log[0], "clear");
    assert_eq!(log[5], "start");
}

#[test]
fn a_loop_of_zero_executes_once() {
    // "A value of 0 executes once."
    assert_eq!(run_text("LOOP 0\nSTART\nEND_LOOP\n"), vec!["start"]);
}

#[test]
fn a_loop_without_an_end_executes_once() {
    assert_eq!(run_text("LOOP 5\nSTART\n"), vec!["start"]);
}

#[test]
fn the_loop_counter_substitutes_for_question_marks() {
    let log = run_text("LOOP 3\nSAVE \"DECAY???.CHN\"\nEND_LOOP\n");
    assert_eq!(
        log,
        vec![
            "save DECAY001.CHN",
            "save DECAY002.CHN",
            "save DECAY003.CHN"
        ]
    );
}

#[test]
fn the_loop_counter_is_available_as_a_variable() {
    let log = run_text("LOOP 2\nDESCRIBE_SAMPLE \"pass $(Loop) of $(Loop1)\"\nEND_LOOP\n");
    assert_eq!(log, vec!["describe pass 0 of 1", "describe pass 1 of 2"]);
}

#[test]
fn nested_loops_are_rejected_with_a_clear_error() {
    let job = Job::parse("LOOP 2\nLOOP 2\nSTART\nEND_LOOP\nEND_LOOP\n").unwrap();
    let mut host = Recorder::default();
    assert!(matches!(
        run(&job, &mut host),
        Err(JobError::NestedLoop { line: 2 })
    ));
}

#[test]
fn file_name_variables_follow_the_last_read() {
    let log = run_text(
        "RECALL \"D:\\USER\\SOIL\\SAM001.SPC\"\nSAVE \"$(FullBase).CHN\"\nDESCRIBE_SAMPLE \"$(ShortBase) in $(FileDir) ext $(FileExt)\"\n",
    );
    assert_eq!(log[1], "save D:\\USER\\SOIL\\SAM001.CHN");
    assert_eq!(log[2], "describe SAM001 in D:\\USER\\SOIL ext SPC");
}

#[test]
fn unknown_variables_are_left_alone() {
    let log = run_text("DESCRIBE_SAMPLE \"keep $(Mystery) as is\"\n");
    assert_eq!(log[0], "describe keep $(Mystery) as is");
}

#[test]
fn control_character_variables_expand() {
    let log = run_text("DESCRIBE_SAMPLE \"a$(CR)$(LF)b\"\n");
    assert_eq!(log[0], "describe a\r\nb");
}

#[test]
fn an_unsupported_command_stops_the_job_with_its_line() {
    // The recorder does not implement SMOOTH.
    let job = Job::parse("START\nSMOOTH\n").unwrap();
    let mut host = Recorder::default();
    let error = run(&job, &mut host).unwrap_err();
    match error {
        JobError::Unsupported { line, ref command } => {
            assert_eq!(line, 2);
            assert_eq!(command, "SMOOTH");
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(host.log, vec!["start"]);
}

#[test]
fn a_host_failure_aborts_the_run() {
    let job = Job::parse("LOOP 100\nSTART\nEND_LOOP\n").unwrap();
    let mut host = Recorder {
        stop_after: Some(3),
        ..Recorder::default()
    };
    assert!(matches!(run(&job, &mut host), Err(JobError::Aborted)));
    assert_eq!(host.log.len(), 4);
}

#[test]
fn the_manual_example_runs_end_to_end() {
    let text = "\
SET_DETECTOR 1
SET_PRESET_CLEAR
SET_PRESET_LIVE 1000
LOOP 2
CLEAR
START
WAIT
FILL_BUFFER
SET_DETECTOR 0
DESCRIBE_SAMPLE \"This is sample ???.\"
SAVE \"DECAY???.CHN\"
REPORT \"PRN\"
SET_DETECTOR 1
END_LOOP
";
    let log = run_text(text);
    assert_eq!(log[0], "detector Some(1)");
    assert_eq!(log[1], "preset clear");
    assert_eq!(log[2], "preset live 1000");
    // Nine commands in the loop body, run twice.
    assert_eq!(log.len(), 3 + 2 * 9);
    assert!(log.contains(&"describe This is sample 001.".to_string()));
    assert!(log.contains(&"save DECAY002.CHN".to_string()));
}
