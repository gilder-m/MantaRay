//! `ortseam` - the headless workbench: convert, analyse, report, automate and
//! acquire without a display.

#![forbid(unsafe_code)]

mod job_host;

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand};
#[cfg(feature = "simulator")]
use ortseam_core::Roi;
use ortseam_core::{
    AnalysisOptions, CalculationSettings, CalibrationPoint, EfficiencyCalibration,
    EnergyCalibration, NuclideLibrary, Spectrum, analyse, auto_calibrate, mark_peaks, peak_info,
    peak_search,
};
#[cfg(feature = "simulator")]
use ortseam_device::{Mcb, Presets, SimulatedMcb, advance};
use ortseam_formats::{AsciiOptions, SpectrumFormat, ascii, library, load_spectrum, save_spectrum};
use ortseam_report::{
    ReportOptions, nuclide_report, print_spectrum, results_csv, roi_report, spectrum_csv, summary,
};

/// Modern multichannel-analyzer workbench for gamma spectroscopy.
#[derive(Parser, Debug)]
#[command(name = "ortseam", version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Summarise spectrum files.
    Info {
        /// Files to describe.
        #[arg(required = true)]
        files: Vec<PathBuf>,
    },
    /// Convert between spectrum formats.
    Convert {
        /// Input file.
        input: PathBuf,
        /// Output file; the format follows its extension.
        output: PathBuf,
        /// Data columns per line for ASCII output.
        #[arg(long, default_value_t = 5)]
        columns: usize,
        /// Leave out the channel numbers in ASCII output.
        #[arg(long)]
        no_channels: bool,
        /// Leave out the header in ASCII output.
        #[arg(long)]
        no_header: bool,
    },
    /// Search for peaks and list them.
    Peaks {
        /// Spectrum file.
        file: PathBuf,
        /// Peak-search sensitivity, 1 (most sensitive) to 5.
        #[arg(long, default_value_t = 3)]
        sensitivity: u32,
        /// Nuclide library for identification.
        #[arg(long)]
        library: Option<PathBuf>,
    },
    /// Write an ROI report.
    Report {
        /// Spectrum file.
        file: PathBuf,
        /// Use the column layout instead of paragraphs.
        #[arg(long)]
        column: bool,
        /// Nuclide library for identification.
        #[arg(long)]
        library: Option<PathBuf>,
        /// Do not mark peaks when the file carries no regions.
        #[arg(long)]
        no_mark_peaks: bool,
        /// Write to a file instead of standard output.
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Print channel contents, seven to a line.
    Print {
        /// Spectrum file.
        file: PathBuf,
        /// First channel.
        #[arg(long)]
        from: Option<usize>,
        /// Last channel.
        #[arg(long)]
        to: Option<usize>,
    },
    /// Identify nuclides and compute activities.
    Analyse {
        /// Spectrum file.
        file: PathBuf,
        /// Nuclide library; the built-in standard library by default.
        #[arg(long)]
        library: Option<PathBuf>,
        /// Constant absolute efficiency, as a fraction.
        #[arg(long)]
        efficiency: Option<f64>,
        /// Sample quantity for per-unit results.
        #[arg(long)]
        quantity: Option<f64>,
        /// Unit of the sample quantity.
        #[arg(long, default_value = "")]
        unit: String,
        /// Report detection limits for library nuclides that were not found.
        #[arg(long)]
        mda: bool,
        /// Write the results as CSV to this file.
        #[arg(long)]
        csv: Option<PathBuf>,
    },
    /// Apply an energy calibration.
    Calibrate {
        /// Spectrum file.
        file: PathBuf,
        /// Calibration points as `channel=energy`, at least two.
        #[arg(
            long = "point",
            required_unless_present = "auto",
            conflicts_with = "auto"
        )]
        points: Vec<String>,
        /// Calibrate automatically from this nuclide's known lines
        /// (e.g. `--auto Eu-152`): its peaks are searched for and matched.
        #[arg(long)]
        auto: Option<String>,
        /// Nuclide library for `--auto`; the built-in standard library by default.
        #[arg(long)]
        library: Option<PathBuf>,
        /// Calibration units.
        #[arg(long, default_value = "keV")]
        units: String,
        /// Where to write the calibrated spectrum.
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Build a nuclide library from an evaluated data export.
    ///
    /// No library is shipped: line energies and emission probabilities belong
    /// to whoever evaluated them. This turns the National Nuclear Data Center's
    /// radiation export into one, keeping every number as evaluated.
    Library {
        /// NNDC radiation export in CSV form. Gunzip a `.csv.gz` first.
        #[arg(long)]
        nndc: PathBuf,
        /// Drop lines below this emission probability, in percent.
        #[arg(long, default_value_t = ortseam_formats::nndc::DEFAULT_MIN_INTENSITY)]
        min_intensity: f64,
        /// Where to write the library.
        #[arg(short, long)]
        output: PathBuf,
    },
    /// Export channel data as CSV.
    Csv {
        /// Spectrum file.
        file: PathBuf,
        /// Output file; standard output by default.
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Run a MAESTRO `.JOB` file.
    Job {
        /// The job file.
        file: PathBuf,
        /// Directory to resolve relative file names against.
        #[arg(long)]
        directory: Option<PathBuf>,
        /// Print each command as it runs.
        #[arg(long)]
        trace: bool,
        /// Instrument to acquire from, as host:port.
        ///
        /// A job that only reads and writes files needs no instrument.
        #[arg(long)]
        detector: Option<String>,
    },
    /// Collect a spectrum from the simulated detector.
    #[cfg(feature = "simulator")]
    Acquire(AcquireArgs),
    /// Serve the simulated detector over TCP, as a network instrument.
    ///
    /// Another ortseam (or anything speaking the SET_/SHOW_ dialect) can then
    /// connect to it from the Detector List - the same path real bench
    /// hardware takes.
    #[cfg(feature = "simulator")]
    Serve {
        /// Address to listen on.
        #[arg(long, default_value = "127.0.0.1:2000")]
        address: String,
        /// Channels of instrument memory.
        #[arg(long, default_value_t = 8192)]
        channels: usize,
    },
}

#[derive(Args, Debug)]
struct AcquireArgs {
    /// Detector number.
    #[arg(long, default_value_t = 1)]
    detector: u16,
    /// Live-time preset in seconds.
    #[arg(long)]
    live: Option<f64>,
    /// Real-time preset in seconds.
    #[arg(long)]
    real: Option<f64>,
    /// Memory size in channels.
    #[arg(long, default_value_t = 8192)]
    channels: usize,
    /// Energy per channel in keV.
    #[arg(long, default_value_t = 0.5)]
    gain: f64,
    /// Random seed, so a run can be repeated.
    #[arg(long, default_value_t = 0x5EED)]
    seed: u64,
    /// Where to save the spectrum.
    #[arg(short, long)]
    output: Option<PathBuf>,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Info { files } => info(&files),
        Commands::Convert {
            input,
            output,
            columns,
            no_channels,
            no_header,
        } => convert(&input, &output, columns, !no_channels, !no_header),
        Commands::Peaks {
            file,
            sensitivity,
            library,
        } => peaks(&file, sensitivity, library.as_deref()),
        Commands::Report {
            file,
            column,
            library,
            no_mark_peaks,
            output,
        } => report(
            &file,
            column,
            library.as_deref(),
            !no_mark_peaks,
            output.as_deref(),
        ),
        Commands::Print { file, from, to } => {
            let spectrum = load(&file)?;
            let range = match (from, to) {
                (Some(low), Some(high)) => Some((low, high)),
                (Some(low), None) => Some((low, spectrum.len().saturating_sub(1))),
                (None, Some(high)) => Some((0, high)),
                (None, None) => None,
            };
            print!("{}", print_spectrum(&spectrum, range));
            Ok(())
        }
        Commands::Library {
            nndc,
            min_intensity,
            output,
        } => build_library(&nndc, min_intensity, &output),
        Commands::Analyse {
            file,
            library,
            efficiency,
            quantity,
            unit,
            mda,
            csv,
        } => analyse_command(
            &file,
            library.as_deref(),
            efficiency,
            quantity,
            &unit,
            mda,
            csv.as_deref(),
        ),
        Commands::Calibrate {
            file,
            points,
            auto,
            library,
            units,
            output,
        } => match auto {
            Some(nuclide) => auto_calibrate_command(
                &file,
                &nuclide,
                library.as_deref(),
                &units,
                output.as_deref(),
            ),
            None => calibrate(&file, &points, &units, output.as_deref()),
        },
        Commands::Csv { file, output } => {
            let text = spectrum_csv(&load(&file)?);
            write_out(output.as_deref(), &text)
        }
        Commands::Job {
            file,
            directory,
            trace,
            detector,
        } => {
            let instrument = job_instrument(detector.as_deref())?;
            job_host::run_job_file(&file, directory.as_deref(), trace, instrument)
        }
        #[cfg(feature = "simulator")]
        Commands::Acquire(args) => acquire(args),
        #[cfg(feature = "simulator")]
        Commands::Serve { address, channels } => serve(&address, channels),
    }
}

/// The instrument a job acquires from.
///
/// A named address is connected to. With nothing named, a simulator build
/// supplies its simulator and a shipped build says plainly that it has no
/// instrument, rather than running the job against invented counts.
fn job_instrument(address: Option<&str>) -> Result<Option<ortseam_device::AnyMcb>> {
    if let Some(address) = address {
        let transport =
            ortseam_device::TcpTransport::connect(address, std::time::Duration::from_secs(5))
                .with_context(|| format!("connecting to {address}"))?;
        let remote = ortseam_device::RemoteMcb::connect(Box::new(transport), 1, "JOB")
            .with_context(|| format!("opening the instrument at {address}"))?;
        eprintln!("job instrument: {address}");
        return Ok(Some(remote.into()));
    }
    // Nothing named: a simulator build lends its simulator, a shipped build
    // attaches nothing. Either way a job that only moves files runs, and one
    // that acquires fails by name when it reaches the first START.
    #[cfg(feature = "simulator")]
    {
        Ok(Some(SimulatedMcb::new(1, "SIM-01").into()))
    }
    #[cfg(not(feature = "simulator"))]
    {
        Ok(None)
    }
}

/// Serves the simulator as a network instrument, ticking its clock between
/// commands so a connected display sees a live count.
#[cfg(feature = "simulator")]
fn serve(address: &str, channels: usize) -> Result<()> {
    use std::sync::{Arc, Mutex};
    let listener =
        std::net::TcpListener::bind(address).with_context(|| format!("listening on {address}"))?;
    let instrument: Arc<Mutex<SimulatedMcb>> = Arc::new(Mutex::new(
        SimulatedMcb::builder(1, "SERVED-01")
            .channels(channels)
            .build(),
    ));
    // The clock: real seconds advance the served instrument - idle too,
    // because the automatic tuning routines (Optimize, pole zero) run on the
    // instrument's clock while nothing is counting. Gating this on is_active
    // left a served START_PZ spinning until an acquisition started.
    let clock = Arc::clone(&instrument);
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(std::time::Duration::from_millis(200));
            let mut mcb = clock.lock().expect("instrument lock");
            let _ = advance(&mut *mcb, 0.2);
        }
    });
    eprintln!("serving a {channels}-channel simulated MCB on {address} (Ctrl+C stops it)");
    ortseam_device::server::serve(listener, instrument).context("serving")?;
    Ok(())
}

fn load(path: &Path) -> Result<Spectrum> {
    load_spectrum(path).with_context(|| format!("reading {}", path.display()))
}

/// Turns an evaluated radiation export into a nuclide library.
///
/// The export is the National Nuclear Data Center's, over the ENSDF
/// evaluations. Nothing here invents a number: every energy and emission
/// probability in the result is the evaluated one, and the library records
/// where it came from so a report can cite it.
fn build_library(export: &Path, min_intensity: f64, output: &Path) -> Result<()> {
    let text =
        std::fs::read_to_string(export).with_context(|| format!("reading {}", export.display()))?;
    let built = ortseam_formats::nndc::build(&text, min_intensity)
        .map_err(|error| anyhow::anyhow!("{error}"))?;
    let lines: usize = built
        .library
        .nuclides
        .iter()
        .map(|nuclide| nuclide.peaks.len())
        .sum();
    library::save(&built.library, output)
        .with_context(|| format!("writing {}", output.display()))?;
    eprintln!("{}", built.provenance);
    eprintln!(
        "read {} rows, kept {} lines: {} nuclides, {lines} lines written to {}",
        built.rows_read,
        built.lines_kept,
        built.library.len(),
        output.display()
    );
    Ok(())
}

fn load_library(path: Option<&Path>) -> Result<NuclideLibrary> {
    match path {
        Some(path) => {
            library::load(path).with_context(|| format!("reading library {}", path.display()))
        }
        // No library is shipped: nuclide data belongs to whoever evaluated
        // it, and a hand-entered table with no provenance is worse than none.
        None => Ok(NuclideLibrary::new("")),
    }
}

fn write_out(path: Option<&Path>, text: &str) -> Result<()> {
    match path {
        Some(path) => {
            std::fs::write(path, text).with_context(|| format!("writing {}", path.display()))?;
            eprintln!("wrote {}", path.display());
        }
        None => print!("{text}"),
    }
    Ok(())
}

fn info(files: &[PathBuf]) -> Result<()> {
    for path in files {
        let spectrum = load(path)?;
        println!("{}", path.display());
        println!("  {}", summary(&spectrum));
        if let Some(start) = spectrum.start_time {
            println!("  acquired  {start}");
        }
        if !spectrum.detector_description.trim().is_empty() {
            println!("  detector  {}", spectrum.detector_description.trim());
        }
        if !spectrum.sample_description.trim().is_empty() {
            println!("  sample    {}", spectrum.sample_description.trim());
        }
        if let Some(cal) = &spectrum.energy_calibration {
            println!(
                "  energy    {:.6} + {:.6}*ch + {:.3e}*ch^2 {}",
                cal.coefficients[0], cal.coefficients[1], cal.coefficients[2], cal.units
            );
            println!(
                "  covers    {:.1} to {:.1} {}",
                cal.energy(0.0),
                cal.energy(spectrum.len().saturating_sub(1) as f64),
                cal.units
            );
        }
        if let Some(shape) = spectrum.shape_calibration {
            println!(
                "  shape     FWHM = {:.4} + {:.3e}*ch + {:.3e}*ch^2 channels",
                shape.coefficients[0], shape.coefficients[1], shape.coefficients[2]
            );
        }
        if !spectrum.rois.is_empty() {
            println!("  regions   {}", spectrum.rois.len());
        }
        if let Some(dead) = spectrum.dead_time_percent() {
            println!("  dead time {dead:.2} %");
        }
    }
    Ok(())
}

fn convert(
    input: &Path,
    output: &Path,
    columns: usize,
    channel_numbers: bool,
    header: bool,
) -> Result<()> {
    let spectrum = load(input)?;
    let format = SpectrumFormat::from_path(output)
        .with_context(|| format!("no writer for {}", output.display()))?;
    if format == SpectrumFormat::Ascii {
        let options = AsciiOptions {
            columns,
            channel_numbers,
            header,
            acquisition_info: header,
        };
        std::fs::write(output, ascii::write(&spectrum, &options))
            .with_context(|| format!("writing {}", output.display()))?;
    } else {
        save_spectrum(&spectrum, output)
            .with_context(|| format!("writing {}", output.display()))?;
    }
    eprintln!(
        "{} -> {} ({})",
        input.display(),
        output.display(),
        format.description()
    );
    Ok(())
}

fn peaks(path: &Path, sensitivity: u32, library_path: Option<&Path>) -> Result<()> {
    let spectrum = load(path)?;
    let settings = CalculationSettings::default().with_sensitivity(sensitivity);
    let found = peak_search(&spectrum, &settings);
    let library = load_library(library_path)?;
    println!("{} peaks in {}", found.len(), path.display());
    match &spectrum.energy_calibration {
        Some(_) => println!(
            "{:>4} {:>10} {:>10} {:>7} {:>11} {:<9} {:>7}",
            "#", "CHANNEL", "ENERGY", "FWHM", "AREA", "NUCLIDE", "DELTA"
        ),
        None => println!("{:>4} {:>10} {:>7} {:>11}", "#", "CHANNEL", "FWHM", "AREA"),
    }
    for (index, peak) in found.iter().enumerate() {
        match &spectrum.energy_calibration {
            Some(cal) => {
                let energy = cal.energy(peak.centroid);
                let matched = library.best_match(energy, 2.0);
                println!(
                    "{:>4} {:>10.2} {:>10.2} {:>7.2} {:>11.0} {:<9} {:>7}",
                    index + 1,
                    peak.centroid,
                    energy,
                    peak.width,
                    peak.net_estimate,
                    matched
                        .as_ref()
                        .map(|m| m.nuclide.name.as_str())
                        .unwrap_or("-"),
                    matched
                        .as_ref()
                        .map(|m| format!("{:+.2}", m.delta))
                        .unwrap_or_else(|| "-".to_string()),
                );
            }
            None => println!(
                "{:>4} {:>10.2} {:>7.2} {:>11.0}",
                index + 1,
                peak.centroid,
                peak.width,
                peak.net_estimate
            ),
        }
    }
    Ok(())
}

fn report(
    path: &Path,
    column: bool,
    library_path: Option<&Path>,
    mark: bool,
    output: Option<&Path>,
) -> Result<()> {
    let mut spectrum = load(path)?;
    let settings = CalculationSettings::default();
    if spectrum.rois.is_empty() && mark {
        let marked = mark_peaks(&mut spectrum, &settings);
        eprintln!("marked {marked} peak(s)");
    }
    let library = load_library(library_path)?;
    let options = if column {
        ReportOptions::column()
    } else {
        ReportOptions::paragraph()
    }
    .with_library(&library);
    write_out(output, &roi_report(&spectrum, &options))
}

fn analyse_command(
    path: &Path,
    library_path: Option<&Path>,
    efficiency: Option<f64>,
    quantity: Option<f64>,
    unit: &str,
    mda: bool,
    csv: Option<&Path>,
) -> Result<()> {
    let mut spectrum = load(path)?;
    let settings = CalculationSettings::default();
    if spectrum.rois.is_empty() {
        let marked = mark_peaks(&mut spectrum, &settings);
        eprintln!("marked {marked} peak(s)");
    }
    let library = load_library(library_path)?;
    // Refused rather than reported empty: "no nuclides found" and "nothing to
    // look for" are different answers, and only one of them is true here.
    if library.is_empty() {
        bail!(
            "no nuclide library: give --library <file.Lib>. ortseam ships none, \
             because nuclide data belongs to whoever evaluated it"
        );
    }
    let mut options = AnalysisOptions {
        settings,
        report_mda_for_absent_nuclides: mda,
        ..AnalysisOptions::default()
    };
    if let Some(value) = efficiency {
        options.efficiency = Some(EfficiencyCalibration::constant(value));
    }
    if let Some(value) = quantity {
        options.sample.quantity = value;
        options.sample.unit = unit.to_string();
    }
    let analysis = analyse(&spectrum, &library, &options);
    print!("{}", nuclide_report(&analysis));
    if let Some(path) = csv {
        std::fs::write(path, results_csv(&analysis))
            .with_context(|| format!("writing {}", path.display()))?;
        eprintln!("wrote {}", path.display());
    }
    Ok(())
}

/// `calibrate --auto`: find the peaks, match them against a nuclide's known
/// lines, and fit - the command-line face of the interface's Auto Calibrate.
fn auto_calibrate_command(
    path: &Path,
    nuclide: &str,
    library_path: Option<&Path>,
    units: &str,
    output: Option<&Path>,
) -> Result<()> {
    let library = load_library(library_path)?;
    let Some(nuclide) = library.iter().find(|n| n.name == nuclide) else {
        bail!(
            "{nuclide} is not in the library; it has {}",
            library
                .iter()
                .map(|n| n.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
    };
    let mut spectrum = load(path)?;
    let settings = CalculationSettings::default();
    if spectrum.rois.is_empty() {
        let marked = mark_peaks(&mut spectrum, &settings);
        eprintln!("marked {marked} peak(s)");
    }
    // Centroids strongest first, which is the order the matcher wants.
    let mut peaks: Vec<(f64, f64)> = spectrum
        .rois
        .iter()
        .filter_map(|roi| peak_info(&spectrum, *roi, &settings).ok())
        .map(|info| (info.centroid, info.net_area))
        .collect();
    peaks.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    let centroids: Vec<f64> = peaks.into_iter().map(|(centroid, _)| centroid).collect();
    let lines: Vec<f64> = nuclide.peaks.iter().map(|peak| peak.energy).collect();
    let result = auto_calibrate(&centroids, &lines, units)
        .map_err(|error| anyhow::anyhow!("auto calibration failed: {error}"))?;
    let calibration = result.calibration;
    println!(
        "calibrated from {} {} lines: {:.6} + {:.6}*ch + {:.6e}*ch^2 {}",
        result.matches.len(),
        nuclide.name,
        calibration.coefficients[0],
        calibration.coefficients[1],
        calibration.coefficients[2],
        calibration.units,
    );
    for point in &result.matches {
        println!(
            "  channel {:>9.2} -> {:>10.3} (library {:.3}, residual {:+.3})",
            point.channel,
            calibration.energy(point.channel),
            point.value,
            calibration.energy(point.channel) - point.value
        );
    }
    spectrum.energy_calibration = Some(calibration);
    if let Some(output) = output {
        save_spectrum(&spectrum, output)
            .with_context(|| format!("writing {}", output.display()))?;
        eprintln!("wrote {}", output.display());
    }
    Ok(())
}

fn calibrate(path: &Path, points: &[String], units: &str, output: Option<&Path>) -> Result<()> {
    let mut parsed = Vec::new();
    for point in points {
        let (channel, energy) = point
            .split_once('=')
            .with_context(|| format!("point {point:?} should look like channel=energy"))?;
        parsed.push(CalibrationPoint::new(
            channel
                .trim()
                .parse()
                .with_context(|| format!("channel in {point:?}"))?,
            energy
                .trim()
                .parse()
                .with_context(|| format!("energy in {point:?}"))?,
        ));
    }
    let calibration = EnergyCalibration::fit_with_units(&parsed, units)
        .map_err(|error| anyhow::anyhow!("{error}"))?;
    let mut spectrum = load(path)?;
    spectrum.energy_calibration = Some(calibration.clone());
    println!(
        "calibration: {:.6} + {:.6}*ch + {:.6e}*ch^2 {} (order {})",
        calibration.coefficients[0],
        calibration.coefficients[1],
        calibration.coefficients[2],
        calibration.units,
        calibration.order()
    );
    for point in &parsed {
        println!(
            "  channel {:>9.2} -> {:>10.3} (entered {:.3}, residual {:+.3})",
            point.channel,
            calibration.energy(point.channel),
            point.value,
            calibration.energy(point.channel) - point.value
        );
    }
    if let Some(output) = output {
        save_spectrum(&spectrum, output)
            .with_context(|| format!("writing {}", output.display()))?;
        eprintln!("wrote {}", output.display());
    }
    Ok(())
}

#[cfg(feature = "simulator")]
fn acquire(args: AcquireArgs) -> Result<()> {
    if args.live.is_none() && args.real.is_none() {
        bail!("give --live or --real so the acquisition knows when to stop");
    }
    let mut detector = SimulatedMcb::builder(args.detector, &format!("SIM-{:02}", args.detector))
        .channels(args.channels)
        .gain_kev_per_channel(args.gain)
        .seed(args.seed)
        .build();
    detector
        .set_presets(Presets {
            live_time: args.live,
            real_time: args.real,
            ..Presets::default()
        })
        .map_err(|error| anyhow::anyhow!("{error}"))?;
    detector
        .start()
        .map_err(|error| anyhow::anyhow!("{error}"))?;

    let mut guard = 0usize;
    while detector.is_active() && guard < 1_000_000 {
        advance(&mut detector, 1.0).map_err(|error| anyhow::anyhow!("{error}"))?;
        guard += 1;
    }
    let status = detector.status();
    println!(
        "collected {} counts in {:.1} s live ({:.1} s real, {:.2} % dead)",
        status.total_counts, status.live_time, status.real_time, status.dead_time_percent
    );

    let mut spectrum = detector.spectrum().clone();
    let settings = CalculationSettings::default();
    let marked = mark_peaks(&mut spectrum, &settings);
    println!("{marked} peak(s) found:");
    let library = NuclideLibrary::new("");
    let regions: Vec<Roi> = spectrum.rois.iter().copied().collect();
    for roi in regions {
        if let Ok(info) = peak_info(&spectrum, roi, &settings) {
            let energy = spectrum
                .energy_calibration
                .as_ref()
                .map(|cal| info.centroid_energy(cal));
            let name = energy
                .and_then(|energy| library.best_match(energy, 2.5))
                .map(|matched| matched.nuclide.name.clone())
                .unwrap_or_else(|| "-".to_string());
            println!(
                "  {:>9.2} keV  net {:>10.0} +/- {:>7.0}  {name}",
                energy.unwrap_or(info.centroid),
                info.net_area,
                info.net_area_uncertainty
            );
        }
    }
    if let Some(output) = args.output {
        save_spectrum(&spectrum, &output)
            .with_context(|| format!("writing {}", output.display()))?;
        eprintln!("wrote {}", output.display());
    }
    Ok(())
}
