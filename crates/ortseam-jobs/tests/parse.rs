//! Parsing MAESTRO `.JOB` files (§6.2 and §6.5 of the manual).

use ortseam_jobs::{BeepSpec, Command, Job, JobError, ListSlice};

fn parse(text: &str) -> Job {
    Job::parse(text).unwrap_or_else(|e| panic!("{e}"))
}

fn only(text: &str) -> Command {
    let job = parse(text);
    assert_eq!(job.len(), 1, "expected one command in {text:?}");
    job.commands()[0].command.clone()
}

#[test]
fn comments_and_blank_lines_are_kept_but_do_nothing() {
    let job = parse("REM this is a comment\n\n   \nSTART\n");
    assert_eq!(job.len(), 2);
    assert!(matches!(job.commands()[0].command, Command::Rem(_)));
    assert!(matches!(job.commands()[1].command, Command::Start));
}

#[test]
fn command_words_are_case_insensitive() {
    assert!(matches!(only("start"), Command::Start));
    assert!(matches!(
        only("Set_Detector 3"),
        Command::SetDetector(Some(3))
    ));
}

#[test]
fn detector_selection_accepts_bare_and_quoted_numbers() {
    assert!(matches!(
        only("SET_DETECTOR 1"),
        Command::SetDetector(Some(1))
    ));
    assert!(matches!(
        only("SET_DETECTOR \"12\""),
        Command::SetDetector(Some(12))
    ));
    assert!(matches!(
        only("SET_DETECTOR 0"),
        Command::SetDetector(Some(0))
    ));
    assert!(matches!(only("SET_DETECTOR"), Command::SetDetector(None)));
    assert!(matches!(only("SET_BUFFER"), Command::SetBuffer));
}

#[test]
fn acquisition_commands_parse() {
    assert!(matches!(only("CLEAR"), Command::Clear));
    assert!(matches!(only("STOP"), Command::Stop));
    assert!(matches!(only("FILL_BUFFER"), Command::FillBuffer));
    assert!(matches!(only("SET_LIST"), Command::SetList));
    assert!(matches!(only("SET_PHA"), Command::SetPha));
    assert!(matches!(only("MARK_PEAKS"), Command::MarkPeaks));
    assert!(matches!(only("SMOOTH"), Command::Smooth));
    assert!(matches!(only("QUIT"), Command::Quit));
    assert!(matches!(only("CLOSEBUFFERS"), Command::CloseBuffers));
    assert!(matches!(only("CLOSEMCBS"), Command::CloseMcbs));
}

#[test]
fn preset_commands_carry_their_values() {
    assert!(matches!(only("SET_PRESET_REAL 300"), Command::SetPresetReal(t) if t == 300.0));
    assert!(matches!(only("SET_PRESET_LIVE 1000"), Command::SetPresetLive(t) if t == 1000.0));
    assert!(matches!(
        only("SET_PRESET_COUNT 5000"),
        Command::SetPresetCount(5000)
    ));
    assert!(matches!(
        only("SET_PRESET_INTEG 20000"),
        Command::SetPresetIntegral(20000)
    ));
    assert!(matches!(only("SET_PRESET_CLEAR"), Command::SetPresetClear));
    match only("SET_PRESET_UNCERTAINTY 1.5,100,200") {
        Command::SetPresetUncertainty { limit, low, high } => {
            assert_eq!((limit, low, high), (1.5, 100, 200));
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn file_commands_take_quoted_names() {
    assert!(
        matches!(only("SAVE \"DECAY001.CHN\""), Command::Save(Some(name)) if name == "DECAY001.CHN")
    );
    assert!(matches!(only("SAVE"), Command::Save(None)));
    assert!(matches!(only("RECALL \"a.chn\""), Command::Recall(name) if name == "a.chn"));
    assert!(
        matches!(only("RECALL_CALIB \"c.chn\""), Command::RecallCalibration(n) if n == "c.chn")
    );
    assert!(
        matches!(only("RECALL_CALIBRATION \"c.chn\""), Command::RecallCalibration(n) if n == "c.chn"),
        "the long spelling is accepted too"
    );
    assert!(matches!(only("RECALL_ROI \"r.roi\""), Command::RecallRoi(n) if n == "r.roi"));
    assert!(matches!(only("SAVE_ROI \"r.roi\""), Command::SaveRoi(n) if n == "r.roi"));
    assert!(matches!(only("REPORT \"out.rpt\""), Command::Report(Some(n)) if n == "out.rpt"));
    assert!(matches!(only("REPORT"), Command::Report(None)));
    assert!(
        matches!(only("LOAD_LIBRARY \"lib.json\""), Command::LoadLibrary(n) if n == "lib.json")
    );
    assert!(matches!(only("EXPORT \"x.asc\""), Command::Export(n) if n == "x.asc"));
    assert!(matches!(only("IMPORT \"x.txt\""), Command::Import(n) if n == "x.txt"));
}

#[test]
fn unquoted_file_names_are_accepted() {
    assert!(matches!(only("SAVE TEST001.CHN"), Command::Save(Some(n)) if n == "TEST001.CHN"));
}

#[test]
fn strip_takes_a_factor_and_an_optional_file() {
    match only("STRIP 0,\"back.chn\"") {
        Command::Strip { factor, file } => {
            assert_eq!(factor, 0.0);
            assert_eq!(file.as_deref(), Some("back.chn"));
        }
        other => panic!("{other:?}"),
    }
    match only("STRIP 1.5") {
        Command::Strip { factor, file } => {
            assert_eq!(factor, 1.5);
            assert!(file.is_none());
        }
        other => panic!("{other:?}"),
    }
    assert!(matches!(only("SET_NAME_STRIP \"b.chn\""), Command::SetNameStrip(n) if n == "b.chn"));
}

#[test]
fn wait_has_four_forms() {
    assert!(matches!(only("WAIT"), Command::Wait(None)));
    assert!(matches!(only("WAIT 2.5"), Command::Wait(Some(t)) if t == 2.5));
    assert!(matches!(only("WAIT \"notepad.exe\""), Command::WaitProgram(p) if p == "notepad.exe"));
    assert!(matches!(only("WAIT_CHANGER"), Command::WaitChanger));
    assert!(matches!(only("WAIT_AUTO"), Command::WaitAuto));
    assert!(matches!(only("WAIT_PZ"), Command::WaitPz));
}

#[test]
fn beep_has_three_forms() {
    assert!(matches!(
        only("BEEP 1000,1000"),
        Command::Beep(BeepSpec::Tone {
            frequency: 1000,
            duration: 1000
        })
    ));
    assert!(matches!(only("BEEP 7"), Command::Beep(BeepSpec::Event(7))));
    assert!(
        matches!(only("BEEP \"alarm.wav\""), Command::Beep(BeepSpec::Sound(s)) if s == "alarm.wav")
    );
}

#[test]
fn lock_and_unlock_carry_credentials() {
    match only("LOCK \"pwd\",\"owner\",\"Station 4\"") {
        Command::Lock {
            password,
            owner,
            name,
        } => {
            assert_eq!(password, "pwd");
            assert_eq!(owner, "owner");
            assert_eq!(name.as_deref(), Some("Station 4"));
        }
        other => panic!("{other:?}"),
    }
    match only("LOCK \"pwd\"") {
        Command::Lock {
            password,
            owner,
            name,
        } => {
            assert_eq!(password, "pwd");
            assert!(owner.is_empty());
            assert!(name.is_none());
        }
        other => panic!("{other:?}"),
    }
    assert!(matches!(only("UNLOCK \"pwd\""), Command::Unlock(p) if p == "pwd"));
    assert!(matches!(only("ASK_PASSWORD"), Command::AskPassword));
}

#[test]
fn set_range_has_both_syntaxes() {
    match only("SET_RANGE \"6/29/2012\",\"14:05:00\",900") {
        Command::SetRange(ListSlice::Absolute {
            date,
            time,
            duration,
        }) => {
            assert_eq!(date, "6/29/2012");
            assert_eq!(time, "14:05:00");
            assert_eq!(duration, 900.0);
        }
        other => panic!("{other:?}"),
    }
    match only("SET_RANGE \"900\",\"900\"") {
        Command::SetRange(ListSlice::Relative { start, duration }) => {
            assert_eq!((start, duration), (900.0, 900.0));
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn loops_and_the_remaining_commands_parse() {
    let job = parse("LOOP 10\nSTART\nEND_LOOP\n");
    assert!(matches!(job.commands()[0].command, Command::Loop(10)));
    assert!(matches!(job.commands()[2].command, Command::EndLoop));
    assert!(matches!(only("LOOP SPECTRA"), Command::LoopSpectra));
    assert!(matches!(only("VIEW \"3\""), Command::View(3)));
    assert!(matches!(only("CHANGE_SAMPLE"), Command::ChangeSample));
    assert!(
        matches!(only("SEND_MESSAGE \"SET_GAIN_FINE 2048\""), Command::SendMessage(m) if m == "SET_GAIN_FINE 2048")
    );
    assert!(
        matches!(only("DESCRIBE_SAMPLE \"soil 12\""), Command::DescribeSample(d) if d == "soil 12")
    );
    assert!(matches!(only("RUN \"prog.exe\""), Command::Run { .. }));
    assert!(matches!(
        only("RUN_MINIMIZED \"prog.exe\""),
        Command::Run {
            minimized: true,
            ..
        }
    ));
    assert!(matches!(only("START_OPTIMIZE"), Command::StartOptimize));
    assert!(matches!(only("START_PZ"), Command::StartPz));
    assert!(matches!(only("STOP_PZ"), Command::StopPz));
    assert!(matches!(only("ZOOM 1"), Command::Zoom(1)));
    match only("ZOOM: 10,20,300,400") {
        Command::ZoomRect {
            x,
            y,
            width,
            height,
        } => assert_eq!((x, y, width, height), (10, 20, 300, 400)),
        other => panic!("{other:?}"),
    }
}

#[test]
fn an_unknown_command_reports_its_line() {
    let error = Job::parse("START\nFLY_TO_THE_MOON\nSTOP\n").unwrap_err();
    match error {
        JobError::UnknownCommand { line, ref word } => {
            assert_eq!(line, 2);
            assert_eq!(word, "FLY_TO_THE_MOON");
        }
        other => panic!("{other:?}"),
    }
    assert!(error.to_string().contains("line 2"));
}

#[test]
fn a_bad_argument_reports_its_line() {
    let error = Job::parse("SET_PRESET_REAL abc\n").unwrap_err();
    assert!(
        matches!(error, JobError::BadArgument { line: 1, .. }),
        "{error:?}"
    );
}

#[test]
fn the_manual_example_parses_completely() {
    // §6.4.1 of the manual, verbatim.
    let text = "\
SET_DETECTOR 1
SET_PRESET_CLEAR
SET_PRESET_LIVE 1000
LOOP 10
CLEAR
START
WAIT
FILL_BUFFER
SET_DETECTOR 0
DESCRIBE_SAMPLE \"This is sample ???.\"
SAVE \"DECAY???.CHN\"
RECALL_ROI \"DECAYPK.ROI\"
REPORT \"PRN\"
SET_DETECTOR 1
END_LOOP
";
    let job = parse(text);
    assert_eq!(job.len(), 15);
    assert_eq!(job.loop_count(), 1);
}
