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
#[cfg(windows)]
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

#[cfg(not(windows))]
fn main() -> std::process::ExitCode {
    eprintln!(
        "ortseam-mcb reaches instruments through ORTEC's library, which exists only \
         as a 32-bit Windows DLL. On this platform the instrument would be reached \
         over USB directly, which is not written yet."
    );
    std::process::ExitCode::FAILURE
}
