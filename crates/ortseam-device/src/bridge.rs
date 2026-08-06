//! An instrument reached through a helper process.
//!
//! ORTEC's library is 32-bit and in-process, so a 64-bit ortseam cannot load it
//! however much it would like to. `ortseam-mcb` is a small executable built for
//! i686 that owns the library and speaks ortseam's dialect on a pipe; this
//! [`Transport`] starts it and carries lines to and from it. Away from Windows
//! the same helper reaches the adapter over libusb instead, and this transport
//! neither knows nor cares: the dialect on the pipe is identical.
//!
//! Nothing above this knows the difference. [`RemoteMcb`](crate::RemoteMcb)
//! drives a socket and drives this with the same code, because both are a
//! command out and a line back.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::Duration;

use crate::error::DeviceError;
use crate::transport::Transport;

/// How long one answer may take before the bridge is declared wedged.
///
/// The slowest real operation - a whole-spectrum read through ORTEC's own
/// library - is around a sixteenth of a second; ten of them is a helper that
/// is not coming back, and blocking forever on it would hang the application
/// (the TCP transport has had the same guard from the start).
const EXCHANGE_TIMEOUT: Duration = Duration::from_secs(10);

/// The helper's name. It sits beside ortseam in a normal installation.
#[cfg(windows)]
pub const BRIDGE_EXECUTABLE: &str = "ortseam-mcb.exe";
/// The same helper away from Windows, where it reaches the adapter over libusb
/// rather than through ORTEC's library - see `docs/ortec-hardware.md`.
#[cfg(not(windows))]
pub const BRIDGE_EXECUTABLE: &str = "ortseam-mcb";

/// Starts a helper without giving it a console window of its own.
///
/// A console program started from a windowed one gets a console, which appears
/// as a black window flashing up beside ortseam. `CREATE_NO_WINDOW` asks
/// Windows not to; everywhere else there is nothing to ask.
pub fn no_console(command: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    let _ = command;
}

/// A helper process, kept alive for as long as the instrument is open.
///
/// The helper's answers arrive through a reader thread and a channel rather
/// than a direct pipe read, because a pipe read cannot be given a timeout: a
/// wedged helper would hang `exchange` - and with it the interface - forever.
pub struct BridgeTransport {
    child: Child,
    writer: Option<ChildStdin>,
    lines: Receiver<std::io::Result<String>>,
    /// How many replies are still owed by commands already given up on.
    ///
    /// The helper answers every command with exactly one line, so a timed-out
    /// exchange leaves one line still on its way. Discarding that many before
    /// believing an answer keeps question and answer together; without it the
    /// late reply becomes the next command's answer and every reading after
    /// it is the previous one's - silently, and for as long as the connection
    /// lasts. A stall long enough to matter needs nothing exotic: a suspended
    /// laptop is past ten seconds on its own.
    owed: usize,
    peer: String,
}

/// Reads the helper's answers for the channel until it closes.
fn read_lines(stdout: ChildStdout, sender: std::sync::mpsc::Sender<std::io::Result<String>>) {
    let mut reader = BufReader::new(stdout);
    loop {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) => return,
            Ok(_) => {
                if sender
                    .send(Ok(line.trim_end_matches(['\r', '\n']).to_string()))
                    .is_err()
                {
                    return;
                }
            }
            Err(error) => {
                let _ = sender.send(Err(error));
                return;
            }
        }
    }
}

impl BridgeTransport {
    /// Starts the bridge on a detector number from ORTEC's configuration.
    ///
    /// `executable` names the helper; `umcbi_dir` is passed on when ORTEC's
    /// libraries are not installed system-wide.
    pub fn start(
        executable: &Path,
        detector: i32,
        umcbi_dir: Option<&Path>,
    ) -> Result<Self, DeviceError> {
        Self::start_pinned(executable, detector, None, umcbi_dir)
    }

    /// The same, naming the adapter to open rather than trusting its position.
    ///
    /// Away from ORTEC's configured detector numbers, `serve N` means the Nth
    /// adapter the bus enumerates - which is a position, not an instrument.
    /// Plug in a second adapter and N can lead somewhere else. A serial names
    /// one adapter and keeps naming it, whatever else is plugged in.
    pub fn start_pinned(
        executable: &Path,
        detector: i32,
        serial: Option<&str>,
        umcbi_dir: Option<&Path>,
    ) -> Result<Self, DeviceError> {
        let mut command = Command::new(executable);
        command.arg("serve").arg(detector.to_string());
        no_console(&mut command);
        if let Some(serial) = serial.map(str::trim).filter(|serial| !serial.is_empty()) {
            command.arg("--device").arg(serial);
        }
        if let Some(directory) = umcbi_dir {
            command.arg("--umcbi-dir").arg(directory);
        }
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            // Left attached, so the helper's own account of what it reached
            // lands wherever ortseam's does rather than vanishing.
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|error| DeviceError::Connection {
                address: executable.display().to_string(),
                detail: format!("could not start the bridge: {error}"),
            })?;
        let writer = child.stdin.take().ok_or_else(|| DeviceError::Connection {
            address: executable.display().to_string(),
            detail: "the bridge has no input".into(),
        })?;
        let stdout = child.stdout.take().ok_or_else(|| DeviceError::Connection {
            address: executable.display().to_string(),
            detail: "the bridge has no output".into(),
        })?;
        let (sender, lines) = std::sync::mpsc::channel();
        std::thread::spawn(move || read_lines(stdout, sender));
        Ok(Self {
            child,
            writer: Some(writer),
            lines,
            owed: 0,
            peer: format!("detector {detector} through the bridge"),
        })
    }

    /// Where the helper is, given the running program's own location.
    ///
    /// Beside ortseam is where an installation puts it; the debug and release
    /// directories of a 32-bit build are where a working copy has it.
    pub fn find_executable() -> Option<PathBuf> {
        let mut candidates = Vec::new();
        if let Ok(current) = std::env::current_exe()
            && let Some(directory) = current.parent()
        {
            candidates.push(directory.join(BRIDGE_EXECUTABLE));
            for profile in ["debug", "release"] {
                candidates.push(
                    directory
                        .join("../../i686-pc-windows-msvc")
                        .join(profile)
                        .join(BRIDGE_EXECUTABLE),
                );
            }
        }
        candidates.into_iter().find(|path| path.exists())
    }
}

impl Transport for BridgeTransport {
    fn exchange(&mut self, command: &str) -> Result<String, DeviceError> {
        let failed = |detail: String| DeviceError::Connection {
            address: self.peer.clone(),
            detail,
        };
        let writer = self
            .writer
            .as_mut()
            .ok_or_else(|| failed("the bridge is closed".into()))?;
        writer
            .write_all(command.as_bytes())
            .and_then(|_| writer.write_all(b"\n"))
            .and_then(|_| writer.flush())
            .map_err(|error| failed(error.to_string()))?;
        loop {
            match self.lines.recv_timeout(EXCHANGE_TIMEOUT) {
                // A line owed to a command that was given up on is that
                // command's answer, not this one's: discard it and keep
                // waiting for the one that belongs here.
                Ok(Ok(_)) if self.owed > 0 => self.owed -= 1,
                Ok(Ok(line)) => return Ok(line),
                Ok(Err(error)) => return Err(failed(error.to_string())),
                Err(RecvTimeoutError::Timeout) => {
                    self.owed += 1;
                    return Err(failed(format!(
                        "no answer to {command:?} within {} seconds",
                        EXCHANGE_TIMEOUT.as_secs()
                    )));
                }
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(failed("the bridge stopped".into()));
                }
            }
        }
    }

    fn peer(&self) -> String {
        self.peer.clone()
    }
}

impl Drop for BridgeTransport {
    fn drop(&mut self) {
        // Closing the pipe ends the helper's read loop, which is how it is meant
        // to stop. It is taken out of the struct so that the close happens here
        // rather than after the wait below, which would never return.
        if let Some(writer) = self.writer.take() {
            drop(writer);
        }
        // A bounded wait: a healthy helper exits the moment its input closes,
        // and a wedged one must not hang the drop - it is killed instead, and
        // the final wait reaps what kill leaves.
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        loop {
            match self.child.try_wait() {
                Ok(Some(_)) | Err(_) => return,
                Ok(None) if std::time::Instant::now() >= deadline => {
                    let _ = self.child.kill();
                    let _ = self.child.wait();
                    return;
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            }
        }
    }
}
