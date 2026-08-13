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
/// mantaray-mcb usb                        list the adapters on the bus
/// mantaray-mcb probe                      open each one and ask what it is
/// mantaray-mcb usbtalk SHOW_VERSION       send one command
/// mantaray-mcb usbspectrum --out live.json  read the spectrum out
/// mantaray-mcb serve [N]                  be an instrument for mantaray, on a pipe
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

#[cfg(not(windows))]
fn direct_main() -> Result<(), String> {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let mut positional: Vec<String> = Vec::new();
    let mut wanted: Option<String> = None;
    let mut index = 0;
    while index < arguments.len() {
        match arguments[index].as_str() {
            "--device" => {
                index += 1;
                wanted = arguments.get(index).cloned();
            }
            // Accepted so that a caller built for the Windows bridge can run
            // this one unchanged; there is no ORTEC library here for it to
            // point at.
            "--umcbi-dir" => {
                index += 1;
                eprintln!("--umcbi-dir means nothing away from Windows; ignored");
            }
            "-h" | "--help" => {
                eprintln!(
                    "mantaray-mcb - the bridge to ORTEC hardware\n\n\
                     USAGE:\n  \
                       mantaray-mcb usb\n  \
                       mantaray-mcb probe\n  \
                       mantaray-mcb usbtalk [--device SERIAL] <command...>\n  \
                       mantaray-mcb usbspectrum [--device SERIAL] [--out FILE]\n  \
                       mantaray-mcb usbfix [--device SERIAL] [--cycle]\n  \
                       mantaray-mcb serve [N] [--device SERIAL]\n\n\
                     On this platform the adapter is reached over libusb, with no\n\
                     vendor driver. If opening it fails with a permission error, a\n\
                     udev rule granting {VENDOR:04x}:{PRODUCT:04x} to your user is what is\n\
                     missing.",
                    VENDOR = direct::VENDOR,
                    PRODUCT = direct::PRODUCT,
                );
                return Ok(());
            }
            other => positional.push(other.to_string()),
        }
        index += 1;
    }
    let command = positional.first().map(String::as_str).unwrap_or("usb");
    match command {
        "usb" => {
            let serials = direct::Device::list()?;
            if serials.is_empty() {
                println!("no DPM-USB adapter on the bus");
            } else {
                println!("{} adapter(s):", serials.len());
                for serial in serials {
                    println!("  {serial}");
                }
            }
            Ok(())
        }
        // Opens every adapter and asks what it is, in the same block shape the
        // Windows bridge prints, because mantaray's scan reads exactly that: a
        // numbered header line, then `model` when the instrument answered or
        // `<why not>` when it did not.
        "probe" => {
            use serve::Instrument;
            let serials = direct::Device::list()?;
            if serials.is_empty() {
                println!("no DPM-USB adapter on the bus");
                return Ok(());
            }
            println!("{} adapter(s) on the bus\n", serials.len());
            for (index, serial) in serials.iter().enumerate() {
                let number = index + 1;
                match direct::Device::open(Some(serial)).and_then(serve::ViaDirect::open) {
                    Ok(instrument) => {
                        let model = instrument.model();
                        if model.is_empty() {
                            println!("  {number}: {serial}");
                            println!("      model    unknown");
                        } else {
                            println!("  {number}: {model} {serial}");
                            println!("      model    {model}");
                        }
                        println!("      channels {}", instrument.channels());
                        println!(
                            "      state    {}",
                            if instrument.is_counting() {
                                "counting"
                            } else {
                                "idle"
                            }
                        );
                    }
                    Err(error) => {
                        println!("  {number}: {serial}");
                        println!("      <{error}>");
                    }
                }
            }
            Ok(())
        }
        // There is no pick list to write on this platform: instruments are
        // found on the bus each time, so configuring is looking.
        "configure" => {
            let serials = direct::Device::list()?;
            if serials.is_empty() {
                return Err(
                    "no DPM-USB adapter on the bus, so there is nothing to configure. Check \
                     that the instrument is powered and plugged in."
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
            let instrument = serve::ViaDirect::open(device)?;
            serve::run(&instrument)
        }
        "usbtalk" | "talk" => {
            let device = direct::Device::open(wanted.as_deref())?;
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
            let device = direct::Device::open(wanted.as_deref())?;
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
            "unknown command {other:?}; try usb, probe, usbtalk, usbspectrum, usbfix, serve or \
             configure"
        )),
    }
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
    let device = direct::Device::open(wanted)?;
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
        let Ok(device) = direct::Device::open(Some(&serial)) else {
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
