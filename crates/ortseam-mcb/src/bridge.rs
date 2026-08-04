//! The Windows bridge, in one place so the crate still builds elsewhere.

use std::process::ExitCode;

use crate::umcbi::Umcbi;
use crate::{dpm, serve, umcbi, usb};

pub fn run() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let mut directory = None;
    let mut positional: Vec<String> = Vec::new();
    let mut index = 0;
    while index < arguments.len() {
        match arguments[index].as_str() {
            "--umcbi-dir" => {
                index += 1;
                directory = arguments.get(index).cloned();
            }
            "-h" | "--help" => {
                usage();
                return ExitCode::SUCCESS;
            }
            other => positional.push(other.to_string()),
        }
        index += 1;
    }
    let command = positional.first().map(String::as_str).unwrap_or("probe");
    let outcome = match command {
        "probe" => probe(directory.as_deref()),
        "talk" => talk(directory.as_deref(), &positional[1..]),
        "dump" => dump(directory.as_deref(), &positional[1..]),
        "serve" => serve_bridge(directory.as_deref(), &positional[1..]),
        "configure" => configure(directory.as_deref(), &positional[1..]),
        "usb" => usb_report(),
        "usbtalk" => usb_talk(positional[1..].to_vec()),
        "usbraw" => usb_raw(positional[1..].to_vec()),
        "usbdump" => usb_dump(positional[1..].to_vec()),
        "usbfix" => usb_fix(positional[1..].to_vec()),
        "usbscan" => usb_scan(positional[1..].to_vec()),
        "usbspectrum" => usb_spectrum(positional[1..].to_vec()),
        "usbspaces" => usb_spaces(positional[1..].to_vec()),
        other => Err(format!(
            "unknown command {other:?}; try probe, configure, usb, talk, dump or serve"
        )),
    };
    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

/// Pushes output out before calling into ORTEC's library, so that a call which
/// takes the process down still leaves the trail that led to it on screen.
fn flush() {
    use std::io::Write;
    let _ = std::io::stdout().flush();
}

fn usage() {
    println!(
        "ortseam-mcb - the 32-bit bridge to ORTEC hardware\n\n\
         USAGE:\n  \
           ortseam-mcb probe [--umcbi-dir DIR]\n  \
           ortseam-mcb talk <detector> <command...> [--umcbi-dir DIR]\n\n\
         --umcbi-dir names the directory holding {}, for a machine where\n\
         ORTEC's libraries are not installed system-wide.",
        umcbi::LIBRARY
    );
}

/// Reports what this machine can reach.
fn probe(directory: Option<&str>) -> Result<(), String> {
    let library = Umcbi::load(directory)?;
    println!("{} loaded", umcbi::LIBRARY);
    // SAFETY: the revision call takes nothing and returns an int.
    println!("UMCBI revision {}", unsafe { (library.get_revision)() });
    if std::env::var("ORTSEAM_MCB_DEBUG").is_ok() {
        // ORTEC's own tracing, which reports what the library is looking for.
        // SAFETY: takes and returns a plain int.
        let previous = unsafe { (library.debug)(9) };
        println!("UMCBI debug level 9 (was {previous})");
    }
    println!("calling MIOStartup...");
    flush();
    library.open()?;
    println!("MIOStartup succeeded");
    for (label, path) in umcbi::local_paths() {
        println!("  {label:<18} {path}");
    }
    // What is physically there, asked of the transports, before what any file
    // claims is there. A pick list copied from another machine names the wrong
    // instruments, and this is what catches that.
    let seen = umcbi::discover();
    if seen.is_empty() {
        println!("\nno instruments answered the local transports");
    } else {
        println!("\ninstruments answering now:");
        for (number, model) in &seen {
            println!("  MCB {number}: {model}");
        }
    }
    let count = library.detector_count()?;
    println!("\n{count} detector(s) in the configured list\n");
    if count == 0 {
        println!(
            "Nothing is configured. ORTEC's MCB Configuration (mcbcon32.exe) assigns\n\
             detector numbers; without it the library has nothing to open."
        );
    }
    for number in 1..=count {
        match library.detector_name(number) {
            Ok(name) => println!("  {number}: {name}"),
            Err(error) => {
                println!("  {number}: <{error}>");
                continue;
            }
        }
        match library.open_detector(number) {
            Ok(handle) => {
                println!("      model    {}", library.model(handle));
                println!("      channels {}", library.length(handle));
                println!(
                    "      state    {}",
                    if library.is_counting(handle) {
                        "counting"
                    } else {
                        "idle"
                    }
                );
                // A command the instrument answers without changing anything.
                match library.command(handle, "SHOW_VERSION") {
                    Ok(reply) => println!("      version  {reply:?}"),
                    Err(error) => println!("      version  <{error}>"),
                }
                // SAFETY: the handle came from open_detector and is still open.
                unsafe { (library.close_detector)(handle) };
            }
            Err(error) => println!("      <{error}>"),
        }
    }
    library.close();
    Ok(())
}

/// Sends one command to a detector and prints the reply.
fn talk(directory: Option<&str>, arguments: &[String]) -> Result<(), String> {
    let (number, rest) = arguments
        .split_first()
        .ok_or("give a detector number and a command")?;
    let number: i32 = number
        .parse()
        .map_err(|_| format!("{number:?} is not a detector number"))?;
    if rest.is_empty() {
        return Err("give a command to send".into());
    }
    let command = rest.join(" ");
    let library = Umcbi::load(directory)?;
    library.open()?;
    let handle = library.open_detector(number)?;
    let reply = library.command(handle, &command);
    // SAFETY: the handle came from open_detector and is still open.
    unsafe { (library.close_detector)(handle) };
    library.close();
    println!("{}", reply?);
    Ok(())
}

/// Reads an instrument's memory and writes it out as a spectrum file.
///
/// This is the whole point of the bridge: what the 926 is holding, in a format
/// ortseam and MAESTRO both read.
fn dump(directory: Option<&str>, arguments: &[String]) -> Result<(), String> {
    let (number, rest) = arguments
        .split_first()
        .ok_or("give a detector number, and --out for where to write it")?;
    let number: i32 = number
        .parse()
        .map_err(|_| format!("{number:?} is not a detector number"))?;
    let output = rest
        .iter()
        .position(|argument| argument == "--out")
        .and_then(|at| rest.get(at + 1))
        .ok_or("give --out FILE")?;

    let library = Umcbi::load(directory)?;
    library.open()?;
    let handle = library.open_detector(number)?;
    let channels = library.length(handle);
    println!(
        "detector {number}: {}, {channels} channels",
        library.model(handle)
    );

    let counts = library.read(handle, 0, channels)?;
    let mut spectrum = ortseam_core::Spectrum::from_counts(counts);
    // The clocks are in twenty-millisecond ticks, which is what the instrument
    // counts in; both were checked against MAESTRO reading the same detector.
    spectrum.live_time = ticks(&library, handle, "SHOW_LIVE")?;
    spectrum.real_time = ticks(&library, handle, "SHOW_TRUE")?;
    spectrum.detector_id = number as u16;
    // The same calibration the bridge hands to ortseam when serving live, so a
    // file written here and a window opened there agree about energies.
    let mcb = library.mcb_number(handle);
    match serve::calibration_for(mcb) {
        Some((a, b, c)) => {
            println!("calibration from MCBLOC32.INI [M{mcb}S01]: {a} {b} {c}");
            spectrum.energy_calibration =
                Some(ortseam_core::EnergyCalibration::new([a, b, c], "keV"));
        }
        None => println!("no calibration stored for [M{mcb}S01]; channels only"),
    }
    // MIOGetStartTime reports the acquisition start as a Unix second, when the
    // instrument has one to report.
    spectrum.start_time = library
        .start_time(handle, chrono::Utc::now().timestamp())
        .and_then(|second| chrono::DateTime::from_timestamp(second, 0))
        .map(|time| time.naive_utc());
    spectrum.sample_description = library.command(handle, "SHOW_VERSION").unwrap_or_default();
    // SAFETY: the handle came from open_detector and is still open.
    unsafe { (library.close_detector)(handle) };

    println!(
        "{} counts over {:.2} s live, {:.2} s real",
        spectrum.total_counts(),
        spectrum.live_time,
        spectrum.real_time
    );
    ortseam_formats::save_spectrum(&spectrum, std::path::Path::new(output))
        .map_err(|error| format!("writing {output}: {error}"))?;
    println!("wrote {output}");
    library.close();
    Ok(())
}

/// One of the instrument's clocks, in seconds.
///
/// The reply is a `$G` record: a ten-digit count of twenty-millisecond ticks
/// followed by a three-digit checksum.
fn ticks(library: &Umcbi, detector: umcbi::Hdet, command: &str) -> Result<f64, String> {
    let reply = library.command(detector, command)?;
    let digits: String = reply.chars().filter(char::is_ascii_digit).collect();
    if digits.len() < 4 {
        return Err(format!(
            "{command} answered {reply:?}, which carries no number"
        ));
    }
    let value: f64 = digits[..digits.len() - 3]
        .parse()
        .map_err(|_| format!("{command} answered {reply:?}"))?;
    Ok(value * 0.02)
}

/// Serves one detector to ortseam over standard input and output.
fn serve_bridge(directory: Option<&str>, arguments: &[String]) -> Result<(), String> {
    let number: i32 = arguments
        .first()
        .ok_or("give a detector number to serve")?
        .parse()
        .map_err(|_| "the detector must be a number")?;
    let library = Umcbi::load(directory)?;
    library.open()?;
    let handle = library.open_detector(number)?;
    // Announced on standard error, which is not the pipe carrying the dialect,
    // so a person watching sees what was reached without disturbing ortseam.
    eprintln!(
        "serving detector {number}: {}, {} channels",
        library.model(handle),
        library.length(handle)
    );
    let outcome = serve::run(&library, handle, number);
    // SAFETY: the handle came from open_detector and is still open.
    unsafe { (library.close_detector)(handle) };
    library.close();
    outcome
}

/// Builds the detector list from what is actually connected.
///
/// ORTEC's MCB Configuration writes this file; so can we, and doing it
/// ourselves is the difference between an application and an add-on. What is
/// already there is kept as a backup first, because somebody else's list may be
/// the only record of instruments not plugged in today.
fn configure(directory: Option<&str>, arguments: &[String]) -> Result<(), String> {
    // The add-in list has to exist before the library is loaded, or there is
    // no transport to discover anything with.
    write_addin_list(directory)?;
    let library = Umcbi::load(directory)?;
    library.open()?;
    let found = umcbi::discover();
    library.close();
    if found.is_empty() {
        return Err(
            "no instruments answered, so there is nothing to configure. Check that the \
             instrument is powered and its driver installed."
                .into(),
        );
    }
    let path = umcbi::local_paths()
        .into_iter()
        .find(|(label, _)| *label == "MCBLOC32.INI")
        .map(|(_, path)| std::path::PathBuf::from(path))
        .and_then(|path| path.parent().map(|parent| parent.join("MCBCON32.INI")))
        .ok_or("the local layer did not say where its configuration lives")?;

    let mut text = String::from("[DETS]\r\n");
    text.push_str(&format!("MAX={}\r\n", found.len()));
    for (index, (mcb, model)) in found.iter().enumerate() {
        let number = index + 1;
        // The name is the instrument's own model until somebody renames it.
        // Inventing a friendlier one would be inventing information.
        text.push_str(&format!("NAME{number:03}={number} {model}\r\n"));
        text.push_str(&format!("ADDR{number:03}={mcb} 1 -1 \r\n"));
        text.push_str(&format!("TYPE{number:03}={model}\r\n"));
        text.push_str(&format!("SNUM{number:03}=\r\n"));
    }
    // The DATE line is not decoration. Without it every MIOOpenDetector fails,
    // and it fails reporting no error at all, which is what made this worth
    // finding out once by bisection and writing down. FEAT lines, by contrast,
    // are genuinely optional: the local layer guesses an instrument's features
    // from its type string when none are given.
    text.push_str("[GENERAL]\r\n");
    text.push_str(&format!("DATE={}\r\n", chrono::Utc::now().timestamp()));

    if path.exists() && !arguments.iter().any(|argument| argument == "--replace") {
        let backup = path.with_extension("INI.before-ortseam");
        if !backup.exists() {
            std::fs::copy(&path, &backup)
                .map_err(|error| format!("keeping a backup at {}: {error}", backup.display()))?;
            println!("kept the previous list at {}", backup.display());
        }
    }
    std::fs::write(&path, &text).map_err(|error| format!("writing {}: {error}", path.display()))?;
    println!(
        "wrote {} with {} instrument(s):",
        path.display(),
        found.len()
    );
    for (index, (mcb, model)) in found.iter().enumerate() {
        println!("  {}: {model} at MCB {mcb}", index + 1);
    }
    Ok(())
}

/// Writes the transport list `mcbloc32.dll` reads, when there is not one.
///
/// This is the file that tells the local layer which add-in DLLs to load; with
/// no `ADDIN` lines it loads none and nothing can answer. An existing file is
/// left exactly as it is, because it also holds per-instrument calibrations
/// that are somebody's measurements.
fn write_addin_list(directory: Option<&str>) -> Result<(), String> {
    let home = match directory {
        Some(directory) => std::path::PathBuf::from(directory),
        None => {
            let root = std::env::var("ProgramData")
                .map_err(|_| "ProgramData is not set, so there is nowhere to put it")?;
            std::path::PathBuf::from(root)
                .join("ORTEC shared")
                .join("UMCBI")
        }
    };
    let path = home.join("MCBLOC32.INI");
    if path.exists() {
        return Ok(());
    }
    // Only the add-ins actually beside the library get listed; naming one that
    // is not there makes the local layer complain on every call.
    let mut text = String::from("[CONFIG]\r\n");
    let mut index = 0;
    for name in [
        "DpmUsbAddIn.dll",
        "USBAddin.dll",
        "DigibaseEAddIn.dll",
        "RS232Addin.dll",
        "TCPAddin.dll",
    ] {
        let candidate = home.join(name);
        if candidate.exists() {
            index += 1;
            text.push_str(&format!("ADDIN{index:02}={}\r\n", candidate.display()));
        }
    }
    if index == 0 {
        return Err(format!(
            "no transport add-ins found in {}, so nothing could be reached",
            home.display()
        ));
    }
    text.push_str("[INSTALL]\r\nDPM=1\r\nPCI=1\r\nPP=1\r\nRUNSERVER=0\r\n");
    std::fs::create_dir_all(&home)
        .map_err(|error| format!("making {}: {error}", home.display()))?;
    std::fs::write(&path, &text).map_err(|error| format!("writing {}: {error}", path.display()))?;
    println!("wrote {} with {index} transport(s)", path.display());
    Ok(())
}

/// Reports the instruments reachable without any ORTEC software at all.
///
/// This talks to the driver directly, which is the half of doing without
/// ORTEC's user-mode stack that works today. What it cannot do yet is hold a
/// conversation: the mailbox the ASCII dialect rides on is still to be worked
/// out, so this proves the channel and stops there.
fn usb_report() -> Result<(), String> {
    #[cfg(not(windows))]
    {
        return Err("direct USB access is Windows-only so far; elsewhere it will be libusb".into());
    }
    #[cfg(windows)]
    {
        let paths = usb::devices();
        if paths.is_empty() {
            println!(
                "No instrument is bound to the ORTEC USB driver.
                 Interface {} has nothing registered under it.",
                usb::ORTECUSB_INTERFACE
            );
            return Ok(());
        }
        println!(
            "{} device(s) on the ORTEC USB driver:
",
            paths.len()
        );
        for path in paths {
            println!("  {path}");
            match usb::Device::open(&path) {
                Ok(device) => {
                    let say = |label: &str, value: Result<String, String>| match value {
                        Ok(text) => println!("      {label:<10} {text}"),
                        Err(error) => println!("      {label:<10} <{error}>"),
                    };
                    say("name", device.name());
                    say("friendly", device.friendly_name());
                    say(
                        "endpoints",
                        device.endpoint_count().map(|count| count.to_string()),
                    );
                    // Reading the descriptors through the driver is behind a
                    // flag because it can block: this driver appears to want
                    // overlapped I/O for a transfer, and a plain synchronous
                    // call waits for an answer that never comes. The endpoint
                    // count above is what the driver already knows and is
                    // enough to be going on with.
                    if std::env::args().any(|argument| argument == "--descriptors") {
                        match device.configuration_descriptor() {
                            Ok(descriptor) => {
                                for endpoint in usb::endpoints(&descriptor) {
                                    println!(
                                        "      endpoint   0x{:02X} {} {}, up to {} bytes",
                                        endpoint.address,
                                        if endpoint.is_in() { "in " } else { "out" },
                                        endpoint.kind_name(),
                                        endpoint.max_packet
                                    );
                                }
                            }
                            Err(error) => println!("      endpoints  <{error}>"),
                        }
                    }
                    say(
                        "speed",
                        device.speed().map(|speed| match speed {
                            0 => "low".to_string(),
                            1 => "full".to_string(),
                            2 => "high".to_string(),
                            3 => "super".to_string(),
                            other => format!("{other}"),
                        }),
                    );
                }
                Err(error) => println!("      <{error}>"),
            }
        }
        Ok(())
    }
}

/// Sends one command straight over USB, with no ORTEC library loaded.
///
/// The proof that the decoded protocol is right: ask for a version and see
/// `$F0926-001` come back, the same answer ORTEC's own library gets.
#[cfg(windows)]
fn usb_talk(mut arguments: Vec<String>) -> Result<(), String> {
    let device = pick_usb_device(&mut arguments)?;
    let command = arguments.join(" ");
    if command.is_empty() {
        return Err("give a command to send".into());
    }
    let reply = crate::speak(&device, &command)?;
    println!("{command} -> {reply:?}");
    Ok(())
}

/// Reads a stretch of the instrument's dual-port memory and shows it.
///
/// `ortseam-mcb usbdump 0x0200 64` reads sixty-four bytes from that offset.
/// This is how the memory map was surveyed, and it stays because a hexdump of
/// what the instrument actually holds is the first thing worth having when a
/// reading looks wrong.
#[cfg(windows)]
fn usb_dump(mut arguments: Vec<String>) -> Result<(), String> {
    let number = |text: &str| -> Result<u32, String> {
        let text = text.trim();
        let parsed = match text.strip_prefix("0x") {
            Some(hex) => u32::from_str_radix(hex, 16),
            None => text.parse(),
        };
        parsed.map_err(|_| format!("{text:?} is not a number"))
    };
    // `--space N` reads a space other than the eight a DPM-USB answers for.
    let mut space = None;
    if let Some(at) = arguments.iter().position(|argument| argument == "--space") {
        arguments.remove(at);
        if at >= arguments.len() {
            return Err("--space needs a number".into());
        }
        let value = arguments.remove(at);
        space = Some(u8::try_from(number(&value)?).map_err(|_| "a space is one byte")?);
    }
    let device = pick_usb_device(&mut arguments)?;
    let offset = number(arguments.first().map(String::as_str).unwrap_or("0"))?;
    let length = number(arguments.get(1).map(String::as_str).unwrap_or("64"))? as usize;
    let offset = u16::try_from(offset).map_err(|_| "the address space is sixteen bits")?;

    let memory = dpm::Dpm::new(&device);
    let bytes = match space {
        None => memory.read_all(offset, length)?,
        Some(space) => memory.read_block_in(space, offset, length)?,
    };
    for (row, chunk) in bytes.chunks(16).enumerate() {
        let at = offset as usize + row * 16;
        let hex: Vec<String> = chunk.iter().map(|byte| format!("{byte:02x}")).collect();
        let text: String = chunk
            .iter()
            .map(|byte| {
                if (0x20..0x7f).contains(byte) {
                    *byte as char
                } else {
                    '.'
                }
            })
            .collect();
        println!("{at:#06x}  {:<47}  |{text}|", hex.join(" "));
    }
    Ok(())
}

/// Opens an instrument bound to the driver.
///
/// With more than one on the machine the first is only a guess, so a bare
/// number in the arguments picks by position and anything else by a piece of
/// the path - the serial number being the natural piece to give.
#[cfg(windows)]
fn pick_usb_device(arguments: &mut Vec<String>) -> Result<usb::Device, String> {
    let paths = usb::devices();
    if paths.is_empty() {
        return Err("no instrument is bound to the driver".into());
    }
    let mut wanted = None;
    if let Some(at) = arguments.iter().position(|a| a == "--device") {
        arguments.remove(at);
        if at >= arguments.len() {
            return Err("--device needs a number or a serial".into());
        }
        wanted = Some(arguments.remove(at));
    }
    let path = match wanted {
        None => paths[0].clone(),
        // A serial number is digits too, so a match against the paths comes
        // first and a position is only a position when nothing contains it.
        Some(which) => match paths.iter().find(|p| p.contains(&which)) {
            Some(path) => path.clone(),
            None => match which.parse::<usize>() {
                Ok(index) if index >= 1 && index <= paths.len() => paths[index - 1].clone(),
                _ => {
                    return Err(format!(
                        "{which:?} is not in any device path, and there is no device \
                         numbered {which} of the {}",
                        paths.len()
                    ));
                }
            },
        },
    };
    println!("{path}");
    usb::Device::open(&path)
}

/// Brings a stuck adapter back without touching the cable.
///
/// Tries the gentlest thing first and works up: ask, clear the pipes and ask
/// again, and finally cycle the port - a replug in software - and ask once
/// more. Each stage reports, so what it took is on record.
#[cfg(windows)]
fn usb_fix(mut arguments: Vec<String>) -> Result<(), String> {
    let cycle = match arguments.iter().position(|argument| argument == "--cycle") {
        Some(at) => {
            arguments.remove(at);
            true
        }
        None => false,
    };
    let device = pick_usb_device(&mut arguments)?;

    // Answering is not enough: an adapter whose reply stream has slipped
    // answers every question with the answer to the one before, so each reply
    // here has to be the right kind of answer, twice in a row.
    let sane = |device: &usb::Device| {
        (0..2).all(|_| {
            crate::speak(device, "SHOW_VERSION")
                .map(|reply| reply.starts_with("$F"))
                .unwrap_or(false)
        })
    };

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
    // Cycling the port is a replug in software, and a replug can fail to plug
    // back in: the adapter leaves the bus and only comes back when Windows
    // enumerates it again, which an ordinary user cannot make happen. One
    // adapter here has been sitting unplugged-in-software since. So the last
    // resort is asked for, not taken.
    if !cycle {
        return Err("clearing the pipes was not enough. `usbfix --cycle` will \
                    cycle the port, which is a replug in software - but if the \
                    adapter does not come back, only a real replug or an \
                    administrator can recover it"
            .into());
    }
    println!("still wrong; cycling the port (a replug, in software)...");
    let path = device.path().to_string();
    device.cycle_port()?;
    drop(device);
    // Re-enumeration takes a moment, and the path stays the same because the
    // serial number does.
    for attempt in 1..=20 {
        std::thread::sleep(std::time::Duration::from_millis(500));
        let Ok(device) = usb::Device::open(&path) else {
            continue;
        };
        if sane(&device) {
            println!("recovered by cycling the port, after {} ms", attempt * 500);
            return Ok(());
        }
    }
    Err(
        "the adapter did not come back; the cable and the instrument's power \
         are what is left to check"
            .into(),
    )
}

/// Walks the adapter's address space looking for where the spectrum lives.
///
/// `ortseam-mcb usbscan` reads a block from every offset in turn and reports
/// what came back, so a region that holds channel counts stands out from one
/// that holds mailbox text or filler. It runs the whole walk in one process on
/// purpose: each frame's answer has to be collected before the next is sent,
/// and a scan spread across separate runs reads every reply one question late.
#[cfg(windows)]
fn usb_scan(mut arguments: Vec<String>) -> Result<(), String> {
    let device = pick_usb_device(&mut arguments)?;
    let memory = dpm::Dpm::new(&device);
    const WINDOW: usize = 256;

    let version = crate::speak(&device, "SHOW_VERSION")?;
    println!("talking to {version:?}");

    println!("  offset  first bytes                                       what it looks like");
    let mut previous: Option<Vec<u8>> = None;
    let mut offset: u32 = 0;
    while offset < 0x1_0000 {
        let at = offset as u16;
        match memory.read_block(at, WINDOW) {
            Err(error) => {
                println!("  {at:#06x}  {error}");
                previous = None;
            }
            Ok(bytes) => {
                let head: String = bytes[..16]
                    .iter()
                    .map(|byte| format!("{byte:02x} "))
                    .collect();
                let mirrors = previous.as_deref() == Some(&bytes[..]);
                println!("  {at:#06x}  {head} {}", describe(&bytes, mirrors));
                previous = Some(bytes);
            }
        }
        offset += 0x400;
    }
    Ok(())
}

/// Sweeps the address-space byte, looking for one that holds the spectrum.
///
/// The window space eight reaches is eight kilobytes of mailbox and status,
/// and a whole spectrum demonstrably never crosses it - reading all 8192
/// channels through ORTEC's own library leaves it byte-for-byte unchanged. So
/// either the counts arrive some other way, or they live in another space.
/// This asks all 256 of them, once, in order, in one process.
#[cfg(windows)]
fn usb_spaces(mut arguments: Vec<String>) -> Result<(), String> {
    let device = pick_usb_device(&mut arguments)?;
    let memory = dpm::Dpm::new(&device);
    let version = crate::speak(&device, "SHOW_VERSION")?;
    println!("talking to {version:?}");

    const WINDOW: usize = 64;
    let mut known: Vec<(u8, Vec<u8>)> = Vec::new();
    let mut silent = Vec::new();
    for space in 0..=u8::MAX {
        match memory.read_block_in(space, 0, WINDOW) {
            Err(_) => silent.push(space),
            Ok(bytes) => {
                let same = known.iter().find(|(_, seen)| *seen == bytes);
                match same {
                    Some((first, _)) => println!("  space {space:#04x}  same as {first:#04x}"),
                    None => {
                        println!(
                            "  space {space:#04x}  {}",
                            bytes[..16]
                                .iter()
                                .map(|byte| format!("{byte:02x} "))
                                .collect::<String>()
                        );
                        known.push((space, bytes));
                    }
                }
            }
        }
    }
    println!(
        "\n{} space(s) answered with {} distinct block(s); {} said nothing",
        256 - silent.len(),
        known.len(),
        silent.len()
    );
    Ok(())
}

/// Says what a block of memory looks like, in a word or two.
#[cfg(windows)]
fn describe(bytes: &[u8], mirrors_previous: bool) -> String {
    if bytes.iter().all(|byte| *byte == 0) {
        return "all zero".into();
    }
    let printable = bytes
        .iter()
        .filter(|byte| (0x20..0x7f).contains(*byte))
        .count();
    let distinct = {
        let mut seen = [false; 256];
        for byte in bytes {
            seen[*byte as usize] = true;
        }
        seen.iter().filter(|on| **on).count()
    };
    // The window interleaves: real bytes at even positions, filler between.
    // Counting the odd bytes that are 0x1e or 0xe1 says whether this is the
    // mailbox region or something laid out differently.
    let filler = bytes
        .iter()
        .skip(1)
        .step_by(2)
        .filter(|byte| **byte == 0x1e || **byte == 0xe1)
        .count();
    let mut notes = Vec::new();
    if mirrors_previous {
        notes.push("mirrors the block before it".to_string());
    }
    if filler * 4 > bytes.len() {
        notes.push(format!("interleaved filler ({filler}/{})", bytes.len() / 2));
    }
    if printable * 3 > bytes.len() {
        notes.push(format!("{printable} printable"));
    }
    notes.push(format!("{distinct} distinct byte values"));
    notes.join(", ")
}

/// Reads a whole spectrum in house and writes it out as a spectrum file.
///
/// `ortseam-mcb usbspectrum --out FILE` does what `dump` does, except that it
/// uses none of ORTEC's software - the same single frame MAESTRO sends. The
/// channel count comes from the instrument's own conversion gain, so a 4096-
/// channel setting is read as 4096 channels rather than assumed.
///
/// It checks itself: the counts are summed and compared against the
/// instrument's own `SHOW_INTEGRAL` over the same range, and a disagreement is
/// a failure rather than a warning. A spectrum that is subtly wrong is worse
/// than no spectrum.
#[cfg(windows)]
fn usb_spectrum(mut arguments: Vec<String>) -> Result<(), String> {
    let output = arguments
        .iter()
        .position(|argument| argument == "--out")
        .and_then(|at| arguments.get(at + 1))
        .cloned();
    if let Some(at) = arguments.iter().position(|argument| argument == "--out") {
        arguments.drain(at..=at + 1.min(arguments.len() - 1 - at));
    }
    let device = pick_usb_device(&mut arguments)?;
    let memory = dpm::Dpm::new(&device);

    // `$C` carries the conversion gain first, in five digits: the number of
    // channels the instrument is actually set to use.
    let gain = memory.command("SHOW_GAIN_CONVERSION")?;
    let channels: usize = gain
        .strip_prefix("$C")
        .and_then(|rest| rest.get(..5))
        .and_then(|digits| digits.trim_start_matches('0').parse().ok())
        .ok_or_else(|| format!("SHOW_GAIN_CONVERSION answered {gain:?}"))?;

    let begun = std::time::Instant::now();
    let (counts, regions) = memory.read_spectrum(channels)?;
    let took = begun.elapsed();

    let total: u64 = counts.iter().map(|count| u64::from(*count)).sum();
    let (peak_at, peak) = counts
        .iter()
        .enumerate()
        .max_by_key(|(_, count)| **count)
        .map(|(at, count)| (at, *count))
        .unwrap_or((0, 0));
    println!(
        "{channels} channels in {:.2} s: total {total}, peak {peak} at channel {peak_at}, \
         {} channel(s) in a region of interest",
        took.as_secs_f64(),
        regions.iter().filter(|inside| **inside).count(),
    );

    // The instrument's own arithmetic over the same range, as a check.
    let integral = memory.command(&format!("SHOW_INTEGRAL 0,{channels}"))?;
    let claimed: u64 = integral
        .strip_prefix("$G")
        .and_then(|rest| rest.get(..10))
        .and_then(|digits| digits.parse().ok())
        .ok_or_else(|| format!("SHOW_INTEGRAL answered {integral:?}"))?;
    if claimed != total {
        return Err(format!(
            "the channels sum to {total} but the instrument says {claimed}; \
             the readout is not to be trusted"
        ));
    }
    println!("SHOW_INTEGRAL agrees: {claimed}");

    if let Some(path) = output {
        let mut spectrum =
            ortseam_core::Spectrum::from_counts(counts.iter().map(|c| u64::from(*c)).collect());
        // Clocks are in twenty-millisecond ticks, as everywhere else here.
        if let Ok(status) = memory.command("SHOW_STAT")
            && let Some(rest) = status.strip_prefix("$M")
            && rest.len() >= 20
            && let (Ok(live), Ok(true_time)) =
                (rest[..10].parse::<f64>(), rest[10..20].parse::<f64>())
        {
            spectrum.live_time = live * 0.02;
            spectrum.real_time = true_time * 0.02;
        }
        // Marking each flagged channel merges runs of them into single
        // regions, which is how the instrument meant them to be read.
        for (at, inside) in regions.iter().enumerate() {
            if *inside {
                spectrum.rois.mark(ortseam_core::Roi::new(at, at));
            }
        }
        ortseam_formats::save_spectrum(&spectrum, &path)
            .map_err(|error| format!("writing {path}: {error}"))?;
        println!("wrote {path}");
    }
    Ok(())
}

/// Sends raw bytes on the bulk endpoints, for working the protocol out.
///
/// `ortseam-mcb usbraw 03 01 00 08 02 00 00` writes those bytes to endpoint 1
/// and reads whatever comes back on 0x81. Smaller than a whole command, which
/// makes it the right size for finding out which half is wrong.
#[cfg(windows)]
fn usb_raw(mut arguments: Vec<String>) -> Result<(), String> {
    let device = pick_usb_device(&mut arguments)?;
    let mut out = Vec::new();
    for value in &arguments {
        out.push(
            u8::from_str_radix(value.trim_start_matches("0x"), 16)
                .map_err(|_| format!("{value:?} is not a hex byte"))?,
        );
    }
    if out.is_empty() {
        return Err("give some hex bytes to send".into());
    }
    println!(
        "out 0x01: {}",
        out.iter()
            .map(|b| format!("{b:02x}"))
            .collect::<Vec<_>>()
            .join(" ")
    );
    let written = device.bulk(0x01, &mut out.clone(), 2000)?;
    println!("        wrote {written} byte(s)");
    // Keep reading until the answer runs dry: a reply longer than one packet
    // arrives as several, and a packet left behind turns up at the head of the
    // next probe, disguised as its answer.
    let mut total = 0usize;
    loop {
        let mut reply = vec![0_u8; 4096];
        let got = match device.bulk(0x81, &mut reply, if total == 0 { 2000 } else { 300 }) {
            Ok(got) => got,
            Err(error) if total > 0 => {
                let _ = error;
                break;
            }
            Err(error) => return Err(error),
        };
        if got == 0 {
            break;
        }
        for (row, chunk) in reply[..got].chunks(16).enumerate() {
            println!(
                "in  0x81 +{:04x}: {}",
                total + row * 16,
                chunk
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect::<Vec<_>>()
                    .join(" ")
            );
        }
        total += got;
        if got % 64 != 0 {
            break; // a short packet ends a transfer
        }
    }
    println!("        {total} byte(s) in all");
    Ok(())
}
