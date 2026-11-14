//! The 32-bit bridge between mantaray and real ORTEC hardware.
//!
//! ORTEC's `Mcbcio32.dll` is 32-bit and in-process, so a 64-bit mantaray cannot
//! load it. This executable is built for i686, owns the library, and will
//! eventually carry commands from mantaray over a pipe. For now it probes: it
//! reports what the machine can see, which is the thing worth knowing first and
//! the thing to fall back on when a bench session misbehaves.
//!
//! ```text
//! mantaray-mcb probe                      what is installed, and which detectors
//! mantaray-mcb talk 2 SHOW_VERSION        send one command to detector 2
//! mantaray-mcb dump 2 --out spectrum.Spe  read the spectrum out
//! mantaray-mcb configure                 build the detector list from what is there
//! mantaray-mcb serve 2                    be an instrument for mantaray, on a pipe
//! ```

#[cfg(windows)]
mod bridge;
#[cfg(not(windows))]
mod direct;
mod dpm;
mod journal;
mod mcb927;
// Used by the Windows road only; tested everywhere.
#[cfg_attr(not(windows), allow(dead_code))]
mod pick;
mod serve;
#[cfg(windows)]
mod umcbi;
#[cfg(windows)]
mod usb;

/// On Windows the bridge owns ORTEC's library; elsewhere it speaks libusb.
///
/// ORTEC's library is a 32-bit Windows DLL and there is no version of it for
/// anything else, so away from Windows the instrument is reached over USB
/// directly - see `docs/ortec-hardware.md` and `direct_main` below. The same
/// serve dialect rides the pipe on every platform.
#[cfg(windows)]
fn main() -> std::process::ExitCode {
    bridge::run()
}

/// Sends one command over the bulk endpoints, with no ORTEC library at all.
#[cfg(windows)]
pub fn speak(device: &usb::Device, command: &str) -> Result<String, String> {
    dpm::Dpm::new(device).command(command)
}

/// Away from Windows there is no vendor driver, so the adapter is reached over
/// libusb directly and ORTEC's library never enters into it.
///
/// ```text
/// mantaray-mcb usb                        list the instruments on the bus
/// mantaray-mcb probe                      open each one and ask what it is
/// mantaray-mcb usbtalk SHOW_VERSION       send one command (926)
/// mantaray-mcb usbspectrum --out live.json  read the spectrum out (926)
/// mantaray-mcb serve [N]                  be an instrument for mantaray, on a pipe
/// mantaray-mcb 927status                  what a Model 927 says of itself, read-only
/// mantaray-mcb 927load --firmware PATH    load a Model 927's FPGA
/// ```
#[cfg(not(windows))]
fn main() -> std::process::ExitCode {
    match direct_main() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}

/// A command line, with the options that take a value taken out.
#[cfg(not(windows))]
#[derive(Debug, Default, PartialEq)]
struct Line {
    positional: Vec<String>,
    wanted: Option<String>,
    firmware: Option<String>,
    help: bool,
}

#[cfg(not(windows))]
fn parse_line(arguments: &[String]) -> Result<Line, String> {
    let mut line = Line::default();
    let mut index = 0;
    while index < arguments.len() {
        match arguments[index].as_str() {
            "--device" => {
                index += 1;
                line.wanted = Some(
                    arguments
                        .get(index)
                        .cloned()
                        .ok_or("--device needs a serial number")?,
                );
            }
            "--firmware" => {
                index += 1;
                line.firmware = Some(
                    arguments
                        .get(index)
                        .cloned()
                        .ok_or("--firmware needs the path of the FPGA image")?,
                );
            }
            // Accepted so that a caller built for the Windows bridge can run
            // this one unchanged; there is no ORTEC library here for it to
            // point at.
            "--umcbi-dir" => {
                index += 1;
                eprintln!("--umcbi-dir means nothing away from Windows; ignored");
            }
            "-h" | "--help" => line.help = true,
            other => line.positional.push(other.to_string()),
        }
        index += 1;
    }
    Ok(line)
}

/// Refuses anything after the command but the flags it takes. For the 927's
/// commands, where a serial number typed without `--device` would otherwise
/// be passed over and the first 927 on the bus used instead.
#[cfg(not(windows))]
fn only_flags(command: &str, rest: &[String], allowed: &[&str]) -> Result<(), String> {
    match rest.iter().find(|word| !allowed.contains(&word.as_str())) {
        None => Ok(()),
        Some(word) => Err(format!(
            "{command} does not take {word:?}; name a 927 with --device SERIAL and an image \
             with --firmware PATH"
        )),
    }
}

#[cfg(not(windows))]
fn direct_main() -> Result<(), String> {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    run_line(&arguments)
}

/// Carries out one command line, the words after the program's name.
#[cfg(not(windows))]
fn run_line(arguments: &[String]) -> Result<(), String> {
    let Line {
        positional,
        wanted,
        firmware,
        help,
    } = parse_line(arguments)?;
    if help {
        eprintln!(
            "mantaray-mcb - the bridge to ORTEC hardware\n\n\
             USAGE:\n  \
               mantaray-mcb usb\n  \
               mantaray-mcb probe\n  \
               mantaray-mcb usbtalk [--device SERIAL] <command...>\n  \
               mantaray-mcb usbspectrum [--device SERIAL] [--out FILE]\n  \
               mantaray-mcb usbfix [--device SERIAL] [--cycle]\n  \
               mantaray-mcb serve [N] [--device SERIAL] [--firmware PATH]\n  \
               mantaray-mcb 927status [--device SERIAL]\n  \
               mantaray-mcb 927load --firmware PATH [--device SERIAL] [--force]\n\n\
             On this platform the instrument is reached over libusb, with no\n\
             vendor driver. If opening it fails with a permission error, a\n\
             udev rule granting {} to your user is what is\n\
             missing.\n\n\
             A Model 927 is two instruments, SERIAL-1 and SERIAL-2, one per\n\
             input; 927status and 927load act on both and take the bare\n\
             SERIAL. Its FPGA has to be loaded after every power-up, from\n\
             ORTEC's M927.rbf: name the file with --firmware PATH, in the\n\
             environment as {}=PATH, or copy it to\n\
             {}, and serve and probe load\n\
             it when it is needed. usbtalk, usbspectrum and usbfix are for\n\
             the 926 behind a DPM-USB adapter.",
            direct::looked_for(),
            mcb927::FIRMWARE_VARIABLE,
            mantaray_core::storage::directory()
                .map(|directory| directory.join(mcb927::FIRMWARE_FILE).display().to_string())
                .unwrap_or_else(|| "MantaRay's settings directory".into()),
        );
        return Ok(());
    }
    let firmware = mcb927::firmware_path(
        firmware.as_deref(),
        std::env::var_os(mcb927::FIRMWARE_VARIABLE).as_deref(),
        mantaray_core::storage::directory().as_deref(),
    );
    let command = positional.first().map(String::as_str).unwrap_or("usb");
    match command {
        "usb" => {
            let entries = direct::Device::entries()?;
            if entries.is_empty() {
                println!("no DPM-USB adapter or Model 927 on the bus");
            } else {
                println!("{} adapter(s):", entries.len());
                for entry in entries {
                    match entry.family {
                        direct::Family::DpmUsb => println!("  {}", entry.name),
                        direct::Family::Model927 => {
                            println!("  {}  (Model 927, input {})", entry.name, entry.input);
                        }
                    }
                }
            }
            Ok(())
        }
        // Opens every adapter and asks what it is, in the same block shape the
        // Windows bridge prints, because mantaray's scan reads exactly that: a
        // numbered header line, then `model` when the instrument answered or
        // `<why not>` when it did not.
        //
        // A 927 is two blocks, one per input, numbered as `serve N` counts
        // them. One whose FPGA has no configuration is loaded here when an
        // image was named, exactly as `serve` would on opening it - a scan
        // that reported a powered-up 927 as absent would send the operator
        // looking for a fault that is not one.
        "probe" => {
            let entries = direct::Device::entries()?;
            if entries.is_empty() {
                println!("no DPM-USB adapter or Model 927 on the bus");
                return Ok(());
            }
            println!("{} adapter(s) on the bus\n", entries.len());
            for (index, entry) in entries.iter().enumerate() {
                let number = index + 1;
                let serial = &entry.name;
                let opened: Result<Box<dyn serve::Instrument>, String> =
                    direct::Device::open(Some(serial)).and_then(|device| match device.family() {
                        direct::Family::DpmUsb => serve::ViaDirect::open(device)
                            .map(|instrument| Box::new(instrument) as Box<dyn serve::Instrument>),
                        direct::Family::Model927 => via_927(device, firmware.as_deref())
                            .map(|instrument| Box::new(instrument) as Box<dyn serve::Instrument>),
                    });
                println!("{}", probe_block(number, serial, opened));
            }
            Ok(())
        }
        // There is no pick list to write on this platform: instruments are
        // found on the bus each time, so configuring is looking.
        "configure" => {
            let serials = direct::Device::list()?;
            if serials.is_empty() {
                return Err(
                    "no DPM-USB adapter or Model 927 on the bus, so there is nothing to \
                     configure. Check that the instrument is powered and plugged in."
                        .into(),
                );
            }
            println!(
                "nothing to write: away from Windows the bus is the configuration, and \
                 {} adapter(s) are on it now:",
                serials.len()
            );
            for (index, serial) in serials.iter().enumerate() {
                println!("  {}: {serial}", index + 1);
            }
            Ok(())
        }
        // Serves one instrument to mantaray over standard input and output,
        // exactly as the Windows bridge does: the same dialect rides the same
        // pipe, only the road to the instrument differs.
        "serve" => {
            let device = match wanted.as_deref() {
                Some(wanted) => direct::Device::open(Some(wanted))?,
                None => {
                    let number: usize = match positional.get(1) {
                        None => 1,
                        Some(text) => text
                            .parse()
                            .ok()
                            .filter(|number| *number >= 1)
                            .ok_or_else(|| format!("{text:?} is not an adapter number"))?,
                    };
                    if number == 1 {
                        direct::Device::open(None)?
                    } else {
                        let serials = direct::Device::list()?;
                        let serial = serials.get(number - 1).ok_or_else(|| {
                            format!("adapter {number} of {} is not there", serials.len())
                        })?;
                        direct::Device::open(Some(serial))?
                    }
                }
            };
            match device.family() {
                direct::Family::DpmUsb => serve::run(&serve::ViaDirect::open(device)?),
                direct::Family::Model927 => serve::run(&via_927(device, firmware.as_deref())?),
            }
        }
        "927status" => {
            only_flags(command, &positional[1..], &[])?;
            status_927(wanted.as_deref())
        }
        "927load" => {
            only_flags(command, &positional[1..], &["--force"])?;
            load_927(
                wanted.as_deref(),
                firmware.as_deref(),
                positional.iter().any(|argument| argument == "--force"),
            )
        }
        "usbtalk" | "talk" => {
            let device = direct::Device::open_dpm(wanted.as_deref())?;
            let text = positional[1..].join(" ");
            if text.is_empty() {
                return Err("give a command to send".into());
            }
            eprintln!("adapter {}", device.serial());
            let reply = dpm::Dpm::new(&device).command(&text)?;
            println!("{text} -> {reply:?}");
            Ok(())
        }
        "usbspectrum" | "spectrum" => {
            // Settled before the adapter is touched. A mistyped command line
            // should not cost an instrument round-trip, and finding out that
            // `--out` was given nothing only after reading four thousand
            // channels back is the wrong end to find it out from.
            let output = match positional.iter().position(|argument| argument == "--out") {
                None => None,
                // Asked for and not given. Returning quietly here would read
                // as a spectrum written, and the operator would go looking
                // for a file that was never named.
                Some(at) => Some(
                    positional
                        .get(at + 1)
                        .ok_or("--out needs a file to write to")?,
                ),
            };
            let device = direct::Device::open_dpm(wanted.as_deref())?;
            eprintln!("adapter {}", device.serial());
            let memory = dpm::Dpm::new(&device);
            let gain = memory.command("SHOW_GAIN_CONVERSION")?;
            let channels: usize = gain
                .strip_prefix("$C")
                .and_then(|rest| rest.get(..5))
                .and_then(|digits| digits.trim_start_matches('0').parse().ok())
                .ok_or_else(|| format!("SHOW_GAIN_CONVERSION answered {gain:?}"))?;
            let (counts, regions) = memory.read_spectrum(channels)?;
            let total: u64 = counts.iter().map(|count| u64::from(*count)).sum();
            println!(
                "{channels} channels, total {total}, {} channel(s) in a region of interest",
                regions.iter().filter(|inside| **inside).count()
            );
            let Some(output) = output else {
                return Ok(());
            };
            let mut spectrum = mantaray_core::Spectrum::from_counts(
                counts.iter().map(|c| u64::from(*c)).collect(),
            );
            // Both clocks are in twenty-millisecond ticks, the unit the
            // instrument counts in, and the same one the Windows dump reads.
            spectrum.live_time = ticks(&memory, "SHOW_LIVE")?;
            spectrum.real_time = ticks(&memory, "SHOW_TRUE")?;
            // The model as `probe` prints it, not the raw record: `0926-001`
            // rather than `$F0926-001`. The `$F` is protocol framing, and a
            // file that carries it is showing an operator the wire.
            spectrum.sample_description = memory
                .command("SHOW_VERSION")
                .map(|reply| serve::record_text(&reply))
                .unwrap_or_default();
            // The instrument marks regions a channel at a time; a region is the
            // run those flags make, so the runs are what get marked here. One
            // region per channel would be the same information in a form
            // nothing downstream would recognise as a region.
            let mut runs: Vec<(usize, usize)> = Vec::new();
            for (channel, inside) in regions.iter().enumerate() {
                match (inside, runs.last_mut()) {
                    (false, _) => {}
                    (true, Some(last)) if last.1 + 1 == channel => last.1 = channel,
                    (true, _) => runs.push((channel, channel)),
                }
            }
            spectrum.rois = mantaray_core::RoiSet::from_pairs(runs);
            mantaray_formats::save_spectrum(&spectrum, std::path::Path::new(output))
                .map_err(|error| format!("writing {output}: {error}"))?;
            println!(
                "wrote {output}: {} counts over {:.2} s live, {:.2} s real",
                spectrum.total_counts(),
                spectrum.live_time,
                spectrum.real_time
            );
            Ok(())
        }
        "usbfix" | "fix" => usb_fix(wanted.as_deref(), &positional[1..]),
        other => Err(format!(
            "unknown command {other:?}; try usb, probe, usbtalk, usbspectrum, usbfix, serve, \
             configure, 927status or 927load"
        )),
    }
}

/// One instrument's block of `probe` output, in the shape mantaray's scan
/// reads: a numbered header, then `model` when it answered or `<why not>`
/// when it did not.
#[cfg(not(windows))]
fn probe_block(
    number: usize,
    serial: &str,
    opened: Result<Box<dyn serve::Instrument>, String>,
) -> String {
    let instrument = match opened {
        Ok(instrument) => instrument,
        Err(error) => return format!("  {number}: {serial}\n      <{error}>"),
    };
    let model = instrument.model();
    let mut block = if model.is_empty() {
        format!("  {number}: {serial}\n      model    unknown")
    } else {
        format!("  {number}: {model} {serial}\n      model    {model}")
    };
    block.push_str(&format!("\n      channels {}", instrument.channels()));
    block.push_str(&match instrument.is_counting() {
        Ok(true) => "\n      state    counting".to_string(),
        Ok(false) => "\n      state    idle".to_string(),
        Err(error) => format!("\n      state    unknown ({error})"),
    });
    block
}

/// Opens the input of a 927 that a device handle was opened for.
#[cfg(not(windows))]
fn via_927(
    device: direct::Device,
    firmware: Option<&std::path::Path>,
) -> Result<serve::Via927<direct::Device>, String> {
    let serial = device.serial().to_string();
    let input = mcb927::Input::from_number(device.input())
        .ok_or_else(|| format!("a 927 has no input {}", device.input()))?;
    serve::Via927::open(device, &serial, input, &|| {
        let image = mcb927::firmware_image(firmware)?;
        odd_size_warning(image.len());
        Ok(image)
    })
}

/// Says so when an image is not the size ORTEC's is. Not a refusal: the size
/// is all that is known of the file here, and the FPGA is the judge of it.
#[cfg(not(windows))]
fn odd_size_warning(size: usize) {
    if size != mcb927::FIRMWARE_SIZE {
        eprintln!(
            "warning: the FPGA image is {size} bytes, and ORTEC's M927.rbf is {}; loading it \
             as given",
            mcb927::FIRMWARE_SIZE
        );
    }
}

/// What a Model 927 says of itself. Reads only: the FPGA test, the firmware
/// version, and with a configured FPGA the registers of both inputs.
#[cfg(not(windows))]
fn status_927(wanted: Option<&str>) -> Result<(), String> {
    use mcb927::{Fpga, Input, Mcb927, Register, Shared};
    let device = direct::Device::open_927(wanted)?;
    println!("Model 927 {}", device.serial());
    device.hold(mcb927::PATIENCE, |wire| {
        let mcb = Mcb927::new(wire);
        let fpga = mcb.fpga_status()?;
        match mcb.firmware_version() {
            Ok(version) => println!("  firmware   {version}"),
            Err(error) => println!("  firmware   <{error}>"),
        }
        if fpga == Fpga::NeedsConfiguration {
            println!("  FPGA       needs configuration (load it with 927load, or let serve)");
            return Ok(());
        }
        println!("  FPGA       configured, version {}", mcb.fpga_version()?);
        const SHOWN: [(&str, Register); 11] = [
            ("control", Register::Control),
            ("gain", Register::ConversionGain),
            ("zdt mode", Register::ZdtMode),
            ("presets", Register::PresetControl),
            ("live preset", Register::LivePreset),
            ("real preset", Register::RealPreset),
            ("peak preset", Register::RoiPeakPreset),
            ("integ preset", Register::RoiIntegralPreset),
            ("status", Register::AcquisitionStatus),
            ("live", Register::LiveTime),
            ("real", Register::RealTime),
        ];
        for input in [Input::One, Input::Two] {
            println!("  input {}", input.number());
            for (label, register) in SHOWN {
                let value = mcb.read_register(input, register)?;
                let meaning = match register {
                    Register::ConversionGain if value <= 6 => {
                        format!("  ({} channels)", mcb927::MAX_CHANNELS >> value)
                    }
                    Register::AcquisitionStatus if value & 1 != 0 => "  (counting)".to_string(),
                    Register::AcquisitionStatus => "  (idle)".to_string(),
                    Register::LiveTime
                    | Register::RealTime
                    | Register::LivePreset
                    | Register::RealPreset => {
                        format!("  ({:.2} s)", f64::from(value) * serve::TICK)
                    }
                    _ => String::new(),
                };
                println!("    {label:<13}{value:#010x}{meaning}");
            }
        }
        println!("  the discriminators cannot be read back");
        Ok(())
    })
}

/// Loads a Model 927's FPGA and opens its discriminators, and nothing else.
///
/// A load drops the configuration the FPGA held, so one that is already
/// configured is left alone unless `--force` says otherwise.
#[cfg(not(windows))]
fn load_927(
    wanted: Option<&str>,
    firmware: Option<&std::path::Path>,
    force: bool,
) -> Result<(), String> {
    use mcb927::{Fpga, Mcb927, Shared};
    // Read before the instrument is touched: a wrong path should not cost
    // a claim.
    let image = mcb927::firmware_image(firmware)?;
    odd_size_warning(image.len());
    let device = direct::Device::open_927(wanted)?;
    println!("Model 927 {}", device.serial());
    device.hold(mcb927::OPEN_PATIENCE, |wire| {
        let mcb = Mcb927::new(wire);
        if mcb.fpga_status()? == Fpga::Configured && !force {
            return Err(
                "the FPGA is already configured. Loading it again drops both inputs' settings \
                 and whatever either is counting; add --force to do it anyway"
                    .into(),
            );
        }
        let began = std::time::Instant::now();
        mcb.configure(&image)?;
        println!(
            "loaded {} bytes in {} ms; the FPGA answers configured",
            image.len(),
            began.elapsed().as_millis()
        );
        println!(
            "both inputs' discriminators set to {} and {}",
            mcb927::DEFAULT_LOWER_DISCRIMINATOR,
            mcb927::DEFAULT_UPPER_DISCRIMINATOR
        );
        Ok(())
    })
}

/// One of the instrument's clocks, in seconds.
///
/// The same reading `serve` does, and deliberately through the same record
/// parser: the clocks come back in twenty-millisecond ticks, and a file written
/// here has to agree with a window served from the same instrument.
#[cfg(not(windows))]
fn ticks(memory: &dpm::Dpm<'_>, command: &str) -> Result<f64, String> {
    let reply = memory.command(command)?;
    Ok(serve::record_number(&reply, 'G')? * serve::TICK)
}

/// Puts a wedged adapter back in step, the libusb counterpart of the Windows
/// `usbfix`.
///
/// Two faults are treated, in the order of how much they disturb. An adapter
/// whose reply stream has slipped answers every question with the answer to the
/// one before, and settling the endpoints puts it back. An adapter whose own
/// state machine has stopped will not so much as take a frame, and only a
/// replug reaches that - so `--cycle` is asked for rather than taken, because a
/// device that re-enumerates is briefly a device that is not there.
#[cfg(not(windows))]
fn usb_fix(wanted: Option<&str>, arguments: &[String]) -> Result<(), String> {
    let cycle = arguments.iter().any(|argument| argument == "--cycle");

    // Answering is not enough: an adapter that is one reply behind still
    // answers, so the reply has to be the right *kind* of answer, twice over.
    // Once could be the stale tail of the question before it.
    let sane = |device: &direct::Device| {
        (0..2).all(|_| {
            dpm::Dpm::new(device)
                .command("SHOW_VERSION")
                .map(|reply| reply.starts_with("$F"))
                .unwrap_or(false)
        })
    };

    // Opening deliberately does not settle - see `Device::open` - so this
    // first look is the adapter as it was found, and the only thing that says
    // whether there is anything here to repair. A healthy adapter answers it
    // and goes no further, which matters: the step after this one is the one
    // that can wedge a working adapter.
    // Adapters only: the ladder below ends in a port reset, and a 927 would
    // fail every ASCII question on the way to it.
    let device = direct::Device::open_dpm(wanted)?;
    let serial = device.serial().to_string();
    println!("adapter {serial}");
    if sane(&device) {
        println!("the instrument answers, and correctly; nothing to fix");
        return Ok(());
    }
    println!("wrong or no answers; clearing the pipes...");
    device.settle();
    if sane(&device) {
        println!("recovered by clearing the pipes");
        return Ok(());
    }
    if !cycle {
        return Err(
            "clearing the pipes was not enough. `usbfix --cycle` will cycle the port, which is a \
             replug in software"
                .into(),
        );
    }
    println!("still wrong; cycling the port (a replug, in software)...");
    device.cycle()?;
    // Re-enumeration takes a moment, and the adapter comes back under the same
    // serial number, which is what it is found by here.
    for attempt in 1..=20 {
        std::thread::sleep(std::time::Duration::from_millis(500));
        let Ok(device) = direct::Device::open_dpm(Some(&serial)) else {
            continue;
        };
        if sane(&device) {
            println!("recovered by cycling the port, after {} ms", attempt * 500);
            return Ok(());
        }
    }
    // Which of the two failures this is decides the advice, and they want
    // opposite things. Still on the bus and still wrong is an instrument that
    // needs its power cycled; gone from the bus is the adapter having taken the
    // replug literally and not come back up, and only the cable fixes that.
    let listed = direct::Device::list().unwrap_or_default();
    if listed.iter().any(|found| found.contains(&serial)) {
        return Err(
            "the adapter came back but still answers wrongly; the instrument's own power is what \
             is left to cycle"
                .into(),
        );
    }
    Err(
        "the adapter did not come back onto the bus. Unplug the USB cable and plug it in again - \
         a cycled port that stays down needs a real replug, and nothing in software can reach it"
            .into(),
    )
}

#[cfg(all(test, not(windows)))]
mod tests {
    use super::*;
    use mcb927::double::Fake927;
    use mcb927::{Input, Register};

    /// Instruments as the scan lists them: (number, name).
    type Listed = Vec<(i32, String)>;

    /// The rule mantaray's scan reads a probe by (`InstrumentScan::parse`
    /// in the GUI crate): a numbered header opens a block, and the line
    /// under it says whether the instrument answered. Returns the ones that
    /// answered and the ones that did not.
    fn scan(text: &str) -> (Listed, Listed) {
        let (mut reachable, mut missing) = (Vec::new(), Vec::new());
        let mut pending = None;
        for line in text.lines() {
            let trimmed = line.trim();
            if let Some((head, name)) = trimmed.split_once(": ")
                && let Ok(number) = head.trim().parse::<i32>()
            {
                pending = Some((number, name.trim().to_string()));
                continue;
            }
            let Some(entry) = pending.clone() else {
                continue;
            };
            if trimmed.starts_with("model") {
                reachable.push(entry);
                pending = None;
            } else if trimmed.starts_with('<') {
                missing.push(entry);
                pending = None;
            }
        }
        (reachable, missing)
    }

    fn input_of(fake: Fake927, input: Input) -> Result<Box<dyn serve::Instrument>, String> {
        serve::Via927::open(fake, "9049204", input, &|| mcb927::firmware_image(None))
            .map(|instrument| Box::new(instrument) as Box<dyn serve::Instrument>)
    }

    #[test]
    fn a_927_is_probed_as_two_blocks_the_scan_can_read() {
        let counting = Fake927::configured();
        counting.set(Input::Two, Register::AcquisitionStatus, 1);
        counting.set(Input::Two, Register::ConversionGain, 1);
        let blocks = [
            probe_block(1, "9049204-1", input_of(Fake927::configured(), Input::One)),
            probe_block(2, "9049204-2", input_of(counting, Input::Two)),
        ]
        .join("\n");
        assert_eq!(
            blocks,
            "  1: 927 9049204-1\n      model    927\n      channels 16384\n      state    idle\n  \
             2: 927 9049204-2\n      model    927\n      channels 8192\n      state    counting"
        );
        let (reachable, missing) = scan(&format!("2 adapter(s) on the bus\n\n{blocks}\n"));
        assert_eq!(
            reachable,
            [(1, "927 9049204-1".into()), (2, "927 9049204-2".into())]
        );
        assert!(missing.is_empty());
    }

    #[test]
    fn a_927_with_no_fpga_and_no_image_is_probed_as_not_answering_with_the_reason() {
        let block = probe_block(
            1,
            "9049204-1",
            input_of(Fake927::unconfigured(), Input::One),
        );
        assert!(block.starts_with("  1: 9049204-1\n      <"), "{block}");
        assert!(block.contains("--firmware PATH"), "{block}");
        assert_eq!(block.lines().count(), 2, "the reason stays on one line");
        let (reachable, missing) = scan(&block);
        assert!(reachable.is_empty());
        assert_eq!(missing, [(1, "9049204-1".into())]);
    }

    fn line(words: &[&str]) -> Result<Line, String> {
        let words: Vec<String> = words.iter().map(|word| word.to_string()).collect();
        parse_line(&words)
    }

    #[test]
    fn a_927_command_never_passes_over_a_word_it_does_not_take() {
        // Through the command line itself, so a command that stopped
        // checking would go on to the bus and fail some other way. The image
        // named is not there, so even that could load nothing.
        let run = |words: &[&str]| {
            let words: Vec<String> = words.iter().map(|word| word.to_string()).collect();
            run_line(&words).unwrap_err()
        };
        // A serial typed without --device would otherwise reload whichever
        // 927 is first on the bus.
        let error = run(&["927load", "--firmware", "/nonexistent/M927.rbf", "9049202"]);
        assert!(
            error.starts_with("927load does not take \"9049202\""),
            "{error}"
        );
        let error = run(&[
            "927load",
            "--firmware",
            "/nonexistent/M927.rbf",
            "--force",
            "x",
        ]);
        assert!(error.starts_with("927load does not take \"x\""), "{error}");
        // A second image is not quietly dropped for the environment's.
        let error = run(&["927load", "/new/M927.rbf"]);
        assert!(error.starts_with("927load does not take"), "{error}");
        let error = run(&["927status", "--json"]);
        assert!(
            error.starts_with("927status does not take \"--json\""),
            "{error}"
        );
        // What is meant to work still does.
        let typed = line(&["927load", "--device", "9049202", "--force"]).unwrap();
        assert_eq!(typed.wanted.as_deref(), Some("9049202"));
        assert_eq!(
            only_flags("927load", &typed.positional[1..], &["--force"]),
            Ok(())
        );
    }

    #[test]
    fn a_device_flag_with_nothing_after_it_is_refused() {
        assert_eq!(
            line(&["927load", "--force", "--device"]),
            Err("--device needs a serial number".into())
        );
    }
}
