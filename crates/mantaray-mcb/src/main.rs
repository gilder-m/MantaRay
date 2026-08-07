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
            Ok(())
        }
        other => Err(format!(
            "unknown command {other:?}; try usb, probe, usbtalk, usbspectrum, serve or configure"
        )),
    }
}
