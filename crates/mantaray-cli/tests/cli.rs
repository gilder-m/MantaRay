//! Integration tests that run the built `mantaray` binary.

use std::path::{Path, PathBuf};
use std::process::Command;

fn workspace() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("mantaray-cli-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn run(args: &[&str]) -> (String, String, bool) {
    let output = Command::new(env!("CARGO_BIN_EXE_mantaray"))
        .args(args)
        .output()
        .expect("running mantaray");
    (
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
        output.status.success(),
    )
}

/// Writes a synthetic calibrated spectrum with one clean peak.
fn sample(path: &Path) {
    let mut counts = vec![0f64; 1024];
    for value in counts.iter_mut() {
        *value = 30.0;
    }
    let sigma = 2.5;
    let amplitude = 60_000.0 / (sigma * (2.0 * std::f64::consts::PI).sqrt());
    for (channel, value) in counts.iter_mut().enumerate() {
        let x = (channel as f64 - 661.657) / sigma;
        *value += amplitude * (-0.5 * x * x).exp();
    }
    let mut spectrum = mantaray_core::Spectrum::from_counts(
        counts.into_iter().map(|c| c.round() as u64).collect(),
    );
    spectrum.live_time = 600.0;
    spectrum.real_time = 610.0;
    spectrum.detector_id = 1;
    spectrum.sample_description = "cli test".into();
    spectrum.energy_calibration = Some(mantaray_core::EnergyCalibration::linear(0.0, 1.0));
    mantaray_formats::save_spectrum(&spectrum, path).unwrap();
}

/// A library file for the tests to identify against.
///
/// No library ships with the program - nuclide data belongs to whoever
/// evaluated it - so every command that identifies nuclides is given one, which
/// is also how a real run works. These values are a test fixture with no
/// provenance; a real library is a `.Lib` file or one built with
/// `mantaray library` from an evaluated export.
fn library_file() -> PathBuf {
    let path = workspace().join("test-library.csv");
    std::fs::write(
        &path,
        "\
# Test fixture, not evaluated data.
Cs-137, 949252608, 661.657, 85.1
Co-60, 166344000, 1173.228, 99.85
Co-60, 166344000, 1332.492, 99.9826
K-40, 39390336000000000, 1460.822, 10.66
Eu-152, 426902400, 121.7817, 28.53
Eu-152, 426902400, 244.6974, 7.55
Eu-152, 426902400, 344.2785, 26.59
Eu-152, 426902400, 778.9045, 12.93
Eu-152, 426902400, 964.057, 14.51
Eu-152, 426902400, 1085.837, 10.11
Eu-152, 426902400, 1112.076, 13.67
Eu-152, 426902400, 1408.013, 20.87
",
    )
    .expect("the test library is written");
    path
}

#[test]
fn help_and_version_work() {
    let (stdout, _, ok) = run(&["--help"]);
    assert!(ok);
    assert!(stdout.contains("workbench"), "{stdout}");
    assert!(stdout.contains("report"), "{stdout}");
    // The simulator's commands appear only in a build that has one.
    assert_eq!(
        stdout.contains("acquire"),
        cfg!(feature = "simulator"),
        "{stdout}"
    );
    let (stdout, _, ok) = run(&["--version"]);
    assert!(ok);
    assert!(stdout.contains("mantaray"), "{stdout}");
}

#[test]
fn info_describes_a_spectrum() {
    let dir = workspace();
    let path = dir.join("info.chn");
    sample(&path);
    let (stdout, _, ok) = run(&["info", path.to_str().unwrap()]);
    assert!(ok, "{stdout}");
    assert!(stdout.contains("1024 channels"), "{stdout}");
    assert!(stdout.contains("LT 600.0"), "{stdout}");
    assert!(stdout.contains("cli test"), "{stdout}");
    assert!(stdout.contains("keV"), "{stdout}");
}

#[test]
fn convert_moves_data_between_formats() {
    let dir = workspace();
    let chn = dir.join("convert.chn");
    sample(&chn);
    let spe = dir.join("convert.spe");
    let json = dir.join("convert.json");
    let spc = dir.join("convert.spc");
    let text = dir.join("convert.txt");

    for (from, to) in [(&chn, &spe), (&spe, &json), (&json, &spc), (&spc, &text)] {
        let (_, stderr, ok) = run(&["convert", from.to_str().unwrap(), to.to_str().unwrap()]);
        assert!(ok, "{} -> {}: {stderr}", from.display(), to.display());
    }

    let original = mantaray_formats::load_spectrum(&chn).unwrap();
    for path in [&spe, &json, &spc, &text] {
        let converted = mantaray_formats::load_spectrum(path).unwrap();
        assert_eq!(
            converted.channels,
            original.channels,
            "{} lost data",
            path.display()
        );
    }
}

#[test]
fn convert_honours_the_ascii_switches() {
    let dir = workspace();
    let chn = dir.join("ascii.chn");
    sample(&chn);
    let text = dir.join("ascii-one-column.txt");
    let (_, stderr, ok) = run(&[
        "convert",
        chn.to_str().unwrap(),
        text.to_str().unwrap(),
        "--columns",
        "1",
        "--no-channels",
        "--no-header",
    ]);
    assert!(ok, "{stderr}");
    let written = std::fs::read_to_string(&text).unwrap();
    assert_eq!(written.lines().count(), 1024);
    assert!(
        written
            .lines()
            .all(|line| line.trim().parse::<u64>().is_ok())
    );
}

#[test]
fn peaks_finds_and_identifies_the_line() {
    let dir = workspace();
    let path = dir.join("peaks.chn");
    sample(&path);
    let library = library_file();
    let (stdout, _, ok) = run(&[
        "peaks",
        path.to_str().unwrap(),
        "--library",
        library.to_str().unwrap(),
    ]);
    assert!(ok, "{stdout}");
    assert!(stdout.contains("Cs-137"), "{stdout}");
    assert!(stdout.contains("661."), "{stdout}");
}

#[test]
fn report_writes_both_layouts() {
    let dir = workspace();
    let path = dir.join("report.chn");
    sample(&path);
    let out = dir.join("report.rpt");
    let library = library_file();
    let (_, stderr, ok) = run(&[
        "report",
        path.to_str().unwrap(),
        "--library",
        library.to_str().unwrap(),
        "--output",
        out.to_str().unwrap(),
    ]);
    assert!(ok, "{stderr}");
    let text = std::fs::read_to_string(&out).unwrap();
    assert!(text.contains("ROI # 1"), "{text}");
    assert!(text.contains("Corrected Rate"), "{text}");

    let (stdout, _, ok) = run(&[
        "report",
        path.to_str().unwrap(),
        "--library",
        library.to_str().unwrap(),
        "--column",
    ]);
    assert!(ok);
    assert!(stdout.contains("ROI#"), "{stdout}");
}

#[test]
fn analyse_reports_an_activity_and_writes_csv() {
    let dir = workspace();
    let path = dir.join("analyse.chn");
    sample(&path);
    let csv = dir.join("analyse.csv");
    let (stdout, stderr, ok) = run(&[
        "analyse",
        path.to_str().unwrap(),
        "--library",
        library_file().to_str().unwrap(),
        "--efficiency",
        "0.05",
        "--csv",
        csv.to_str().unwrap(),
    ]);
    assert!(ok, "{stderr}");
    assert!(stdout.contains("Cs-137"), "{stdout}");
    assert!(stdout.contains("Bq"), "{stdout}");
    let text = std::fs::read_to_string(&csv).unwrap();
    assert!(text.starts_with("nuclide,"), "{text}");
    assert!(text.contains("Cs-137"), "{text}");
}

#[test]
fn calibrate_fits_and_saves() {
    let dir = workspace();
    let path = dir.join("calibrate.chn");
    sample(&path);
    let out = dir.join("calibrated.chn");
    let (stdout, stderr, ok) = run(&[
        "calibrate",
        path.to_str().unwrap(),
        "--point",
        "100=200.5",
        "--point",
        "800=1600.5",
        "--output",
        out.to_str().unwrap(),
    ]);
    assert!(ok, "{stderr}");
    assert!(stdout.contains("calibration:"), "{stdout}");
    let calibrated = mantaray_formats::load_spectrum(&out).unwrap();
    let cal = calibrated.energy_calibration.unwrap();
    assert!((cal.energy(100.0) - 200.5).abs() < 1e-3);
    assert!((cal.energy(800.0) - 1600.5).abs() < 1e-3);
}

#[test]
fn calibrate_auto_recovers_the_instrument_gain_from_eu152() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../mantaray-formats/tests/fixtures/eu152_spectra.Spe");
    // The fixtures are real measured spectra and are deliberately kept out of
    // git, so a clean checkout does not have them. Skipping out loud is the
    // only honest thing here: a green run that tested nothing must not look
    // like a green run that tested something.
    if !fixture.exists() {
        eprintln!(
            "skipping: {} is not present. The measured fixtures are not in \
             git; see crates/mantaray-formats/tests/fixtures/README.md.",
            fixture.display()
        );
        return;
    }
    let dir = workspace();
    let out = dir.join("auto-calibrated.spe");
    let (stdout, stderr, ok) = run(&[
        "calibrate",
        fixture.to_str().unwrap(),
        "--auto",
        "Eu-152",
        "--library",
        library_file().to_str().unwrap(),
        "--output",
        out.to_str().unwrap(),
    ]);
    assert!(ok, "{stderr}");
    assert!(stdout.contains("calibrated from"), "{stdout}");
    let calibrated = mantaray_formats::load_spectrum(&out).unwrap();
    let cal = calibrated.energy_calibration.unwrap();
    // The real instrument's gain was 0.3610 keV/ch; the fit found from the
    // peaks alone must land on it.
    assert!(
        (cal.coefficients[1] - 0.3610).abs() < 0.001,
        "gain {}",
        cal.coefficients[1]
    );
}

#[test]
fn calibrate_refuses_a_point_and_auto_together() {
    let dir = workspace();
    let path = dir.join("conflict.chn");
    sample(&path);
    let (_, stderr, ok) = run(&[
        "calibrate",
        path.to_str().unwrap(),
        "--point",
        "100=200.5",
        "--auto",
        "Eu-152",
    ]);
    assert!(!ok, "the two calibration modes are mutually exclusive");
    assert!(stderr.contains("cannot be used with"), "{stderr}");
}

#[test]
fn print_shows_seven_channels_a_line() {
    let dir = workspace();
    let path = dir.join("print.chn");
    sample(&path);
    let (stdout, _, ok) = run(&["print", path.to_str().unwrap(), "--from", "0", "--to", "13"]);
    assert!(ok);
    let data: Vec<&str> = stdout
        .lines()
        .filter(|line| line.trim_start().starts_with(|c: char| c.is_ascii_digit()))
        .collect();
    assert_eq!(data.len(), 2, "{stdout}");
}

#[test]
#[cfg(feature = "simulator")]
fn acquire_collects_from_the_simulator() {
    let dir = workspace();
    // The native format is used here because .Chn carries no region table.
    let out = dir.join("acquired.json");
    let (stdout, stderr, ok) = run(&[
        "acquire",
        "--live",
        "20",
        "--channels",
        "2048",
        "--gain",
        "1.0",
        "--output",
        out.to_str().unwrap(),
    ]);
    assert!(ok, "{stderr}");
    assert!(stdout.contains("collected"), "{stdout}");
    assert!(stdout.contains("peak(s) found"), "{stdout}");
    assert!(stdout.contains("keV"), "{stdout}");
    let spectrum = mantaray_formats::load_spectrum(&out).unwrap();
    assert!(spectrum.total_counts() > 1000);
    assert!(spectrum.live_time >= 20.0);
    assert!(!spectrum.rois.is_empty(), "the marked peaks were saved");
    assert!(spectrum.energy_calibration.is_some());
}

#[test]
#[cfg(feature = "simulator")]
fn acquire_is_repeatable_for_a_given_seed() {
    let dir = workspace();
    let mut totals = Vec::new();
    for run_index in 0..2 {
        let out = dir.join(format!("seeded-{run_index}.json"));
        let (_, stderr, ok) = run(&[
            "acquire",
            "--live",
            "5",
            "--seed",
            "1234",
            "--output",
            out.to_str().unwrap(),
        ]);
        assert!(ok, "{stderr}");
        totals.push(mantaray_formats::load_spectrum(&out).unwrap().channels);
    }
    assert_eq!(totals[0], totals[1], "the same seed must repeat");
}

#[test]
#[cfg(feature = "simulator")]
fn acquire_without_a_preset_is_refused() {
    let (_, stderr, ok) = run(&["acquire"]);
    assert!(!ok);
    assert!(
        stderr.contains("--live") || stderr.contains("--real"),
        "{stderr}"
    );
}

#[test]
#[cfg(feature = "simulator")]
fn a_job_file_runs_end_to_end() {
    let dir = workspace();
    let job = dir.join("run.job");
    std::fs::write(
        &job,
        "\
REM collect two spectra and report on each
SET_DETECTOR 1
SET_PRESET_CLEAR
SET_PRESET_LIVE 15
LOOP 2
CLEAR
START
WAIT
FILL_BUFFER
SET_DETECTOR 0
DESCRIBE_SAMPLE \"job sample ???\"
SAVE \"JOB???.CHN\"
MARK_PEAKS
REPORT \"JOB???.RPT\"
SET_DETECTOR 1
END_LOOP
",
    )
    .unwrap();

    let (_, stderr, ok) = run(&["job", job.to_str().unwrap(), "--trace"]);
    assert!(ok, "{stderr}");
    assert!(stderr.contains("job finished"), "{stderr}");

    for index in 1..=2 {
        let saved = dir.join(format!("JOB00{index}.CHN"));
        let report = dir.join(format!("JOB00{index}.RPT"));
        assert!(saved.exists(), "{} missing", saved.display());
        assert!(report.exists(), "{} missing", report.display());
        let spectrum = mantaray_formats::load_spectrum(&saved).unwrap();
        assert!(spectrum.live_time >= 15.0);
        assert!(spectrum.total_counts() > 0);
        assert_eq!(spectrum.sample_description, format!("job sample 00{index}"));
        let text = std::fs::read_to_string(&report).unwrap();
        assert!(text.contains("ROI #"), "{text}");
    }
}

#[test]
#[cfg(not(feature = "simulator"))]
fn a_job_that_acquires_says_it_needs_an_instrument() {
    // A shipped build has nothing to acquire from. The job must stop at the
    // command that needs an instrument and name what is missing - never run on
    // to write a file of counts that came from nowhere.
    let dir = workspace();
    let job = dir.join("needs-hardware.job");
    std::fs::write(
        &job,
        "SET_PRESET_LIVE 5
START
WAIT
SAVE \"OUT.CHN\"
",
    )
    .unwrap();
    let (_, stderr, ok) = run(&["job", job.to_str().unwrap()]);
    assert!(!ok, "{stderr}");
    assert!(stderr.contains("needs an instrument"), "{stderr}");
    assert!(stderr.contains("--detector"), "{stderr}");
    assert!(
        !dir.join("OUT.CHN").exists(),
        "no spectrum should have been written"
    );
}

#[test]
fn a_job_that_only_moves_files_needs_no_instrument() {
    // The other half of the same rule: file work still runs with nothing
    // attached, so the refusal above is about acquisition, not about jobs.
    let dir = workspace();
    let source = dir.join("source.chn");
    sample(&source);
    let job = dir.join("files-only.job");
    std::fs::write(
        &job,
        format!(
            "SET_DETECTOR 0
RECALL \"{}\"
MARK_PEAKS
REPORT \"FILES.RPT\"
",
            source.file_name().unwrap().to_str().unwrap()
        ),
    )
    .unwrap();
    let (_, stderr, ok) = run(&["job", job.to_str().unwrap()]);
    assert!(ok, "{stderr}");
    assert!(dir.join("FILES.RPT").exists(), "{stderr}");
}

#[test]
fn a_job_with_an_unknown_command_fails_with_its_line() {
    let dir = workspace();
    let job = dir.join("bad.job");
    std::fs::write(&job, "START\nDO_SOMETHING_ODD\n").unwrap();
    let (_, stderr, ok) = run(&["job", job.to_str().unwrap()]);
    assert!(!ok);
    assert!(stderr.contains("line 2"), "{stderr}");
}

/// `RECALL_CALIBRATION` takes a `.Clb`, which is the file that carries one.
///
/// The desktop menu learned to read a `.Clb` before automation did, so a `.JOB`
/// naming one failed outright with "no reader for that extension" - the same
/// command doing two different things depending on who ran it. Both go through
/// the one reader now.
#[test]
fn a_job_recalls_a_calibration_from_a_clb_file() {
    let dir = workspace();
    let spectrum = dir.join("uncalibrated.chn");
    sample(&spectrum);

    // A `.Clb` of the shape MAESTRO writes: six little-endian floats at 0x94.
    let mut bytes = vec![0u8; 1152];
    for (index, value) in [19.1197f32, 0.420412, 3.060_48e-7, 4.5439, 0.0, 0.0]
        .iter()
        .enumerate()
    {
        let at = 0x94 + index * 4;
        bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }
    let calibration = dir.join("detector.Clb");
    std::fs::write(&calibration, &bytes).unwrap();

    let job = dir.join("recall-clb.job");
    std::fs::write(
        &job,
        format!(
            "RECALL \"{}\"\nRECALL_CALIBRATION \"{}\"\nSAVE \"FROM-A-CLB.CHN\"\n",
            spectrum.display(),
            calibration.display()
        ),
    )
    .unwrap();

    let (_, stderr, ok) = run(&["job", job.to_str().unwrap()]);
    assert!(ok, "a .Clb should be recallable from a job: {stderr}");

    let saved = mantaray_formats::load_spectrum(dir.join("FROM-A-CLB.CHN")).unwrap();
    let energy = saved
        .energy_calibration
        .expect("the calibration should have been applied");
    assert!(
        (energy.coefficients[1] - 0.420_412).abs() < 1e-6,
        "got {:?}",
        energy.coefficients
    );
}

/// A file holding no calibration must not wipe the one already in hand.
///
/// The guard the desktop application has, in the path automation takes. A job
/// that quietly replaced a good calibration with nothing would put every energy
/// in every report it went on to write silently wrong.
#[test]
fn a_job_refuses_to_recall_a_calibration_from_a_file_that_has_none() {
    let dir = workspace();
    let spectrum = dir.join("calibrated-source.chn");
    sample(&spectrum);

    // A .Clb of the right length holding nothing at all.
    let empty = dir.join("empty.Clb");
    std::fs::write(&empty, vec![0u8; 1152]).unwrap();

    let job = dir.join("recall-empty.job");
    std::fs::write(
        &job,
        format!(
            "RECALL \"{}\"\nRECALL_CALIBRATION \"{}\"\n",
            spectrum.display(),
            empty.display()
        ),
    )
    .unwrap();

    let (_, stderr, ok) = run(&["job", job.to_str().unwrap()]);
    assert!(
        !ok,
        "a file with no calibration should stop the job rather than empty the calibration: {stderr}"
    );
}
