//! The 32-bit bridge between ortseam and real ORTEC hardware.
//!
//! ORTEC's `Mcbcio32.dll` is 32-bit and in-process, so a 64-bit ortseam cannot
//! load it. This executable is built for i686, owns the library, and will
//! eventually carry commands from ortseam over a pipe. For now it probes: it
//! reports what the machine can see, which is the thing worth knowing first and
//! the thing to fall back on when a bench session misbehaves.
//!
//! ```text
//! ortseam-mcb probe                      what is installed, and which detectors
//! ortseam-mcb talk 2 SHOW_VERSION        send one command to detector 2
//! ortseam-mcb dump 2 --out spectrum.Spe  read the spectrum out
//! ortseam-mcb configure                 build the detector list from what is there
//! ortseam-mcb serve 2                    be an instrument for ortseam, on a pipe
//! ```

#[cfg(windows)]
mod bridge;
#[cfg(not(windows))]
mod direct;
mod dpm;
#[cfg(windows)]
mod serve;
#[cfg(windows)]
mod umcbi;
#[cfg(windows)]
mod usb;

/// On Windows the bridge does its job; elsewhere it says why it cannot.
///
/// ORTEC's library is a 32-bit Windows DLL and there is no version of it for
/// anything else. Reaching the instrument from Linux or macOS means speaking to
/// it over USB directly, which is a different piece of work - see
/// `docs/ortec-hardware.md`. The crate still builds on those platforms so that
/// a workspace build and the test suite are not split in two.
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
/// ortseam-mcb usb                        list the adapters on the bus
/// ortseam-mcb usbtalk SHOW_VERSION       send one command
/// ortseam-mcb usbspectrum --out live.json  read the spectrum out
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
            "-h" | "--help" => {
                eprintln!(
                    "ortseam-mcb - the bridge to ORTEC hardware\n\n\
                     USAGE:\n  \
                       ortseam-mcb usb\n  \
                       ortseam-mcb usbtalk [--device SERIAL] <command...>\n  \
                       ortseam-mcb usbspectrum [--device SERIAL] [--out FILE]\n\n\
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
        "usb" | "probe" => {
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
            "unknown command {other:?}; try usb, usbtalk, usbspectrum"
        )),
    }
}
