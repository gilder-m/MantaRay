//! An instrument reached through a helper process.
//!
//! ORTEC's library is 32-bit and in-process, so a 64-bit mantaray cannot load it
//! however much it would like to. `mantaray-mcb` is a small executable built for
//! i686 that owns the library and speaks mantaray's dialect on a pipe; this
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

/// The helper's name. It sits beside mantaray in a normal installation.
#[cfg(windows)]
pub const BRIDGE_EXECUTABLE: &str = "mantaray-mcb.exe";
/// The same helper away from Windows, where it reaches the adapter over libusb
/// rather than through ORTEC's library - see `docs/ortec-hardware.md`.
#[cfg(not(windows))]
pub const BRIDGE_EXECUTABLE: &str = "mantaray-mcb";

/// Starts a helper without giving it a console window of its own.
///
/// A console program started from a windowed one gets a console, which appears
/// as a black window flashing up beside mantaray. `CREATE_NO_WINDOW` asks
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
    /// The last thing the helper said on standard error.
    ///
    /// Kept so that a helper which dies before answering can be reported by
    /// its own reason rather than by the silence that follows it.
    complaint: std::sync::Arc<std::sync::Mutex<Option<String>>>,
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

/// Passes the helper's standard error on, keeping the last line said.
///
/// Relayed as it arrives, so nothing is swallowed or held back until the end;
/// the copy is only so a failure can quote the reason instead of the silence.
fn relay_stderr(stderr: std::process::ChildStderr, kept: &std::sync::Mutex<Option<String>>) {
    for line in BufReader::new(stderr).lines().map_while(Result::ok) {
        let line = line.trim().to_string();
        if line.is_empty() {
            continue;
        }
        eprintln!("{line}");
        if let Ok(mut kept) = kept.lock() {
            *kept = Some(line);
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

    /// Starts the hub: one bridge process that serves every local detector,
    /// one command at a time.
    ///
    /// One process is the fix for a fault a process per detector cannot
    /// avoid: entries that resolve to the same physical instrument - which
    /// ORTEC's own configuration is happy to hold - had their transactions
    /// interleave at the mailbox, crossing their replies until the spectrum
    /// on screen flashed between snapshots and the adapter wedged. The hub's
    /// single loop answers one detector before reading the next, so there is
    /// nothing to interleave. Individual lanes are [`HubTransport`]s.
    pub fn start_hub(executable: &Path, umcbi_dir: Option<&Path>) -> Result<Self, DeviceError> {
        let mut command = Command::new(executable);
        command.arg("serve").arg("--hub");
        no_console(&mut command);
        if let Some(directory) = umcbi_dir {
            command.arg("--umcbi-dir").arg(directory);
        }
        Self::launch(command, executable, "the local hub".to_string())
    }

    /// Whether the helper is still running.
    ///
    /// The hub outlives any one detector window, so the application asks
    /// before handing out another lane: a lane onto a dead hub would fail
    /// every exchange, and the honest move is a fresh hub instead.
    pub fn alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    /// Closes the helper's input, which is how it is told to finish now.
    ///
    /// For standing the hub down while another bridge process runs: a probe
    /// against a live hub is two processes in transaction with one driver,
    /// which is the exact contention the hub exists to prevent. Lanes still
    /// holding this line keep their handles; their exchanges answer "the
    /// bridge is closed" from here on, which is the truth.
    pub fn park(&mut self) {
        if let Some(writer) = self.writer.take() {
            drop(writer);
        }
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
        Self::launch(
            command,
            executable,
            format!("detector {detector} through the bridge"),
        )
    }

    /// Spawns a configured helper and wires up its three pipes.
    fn launch(mut command: Command, executable: &Path, peer: String) -> Result<Self, DeviceError> {
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            // Piped rather than inherited, but still relayed line by line, so
            // the helper's account of what it reached lands where mantaray's
            // does *and* is available to quote. A helper that refuses to start
            // - "no adapter with X in its serial number" - says so only here,
            // and a window with no console behind it would otherwise show the
            // operator nothing but "the bridge stopped".
            .stderr(Stdio::piped())
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
        let complaint = std::sync::Arc::new(std::sync::Mutex::new(None));
        if let Some(stderr) = child.stderr.take() {
            let kept = std::sync::Arc::clone(&complaint);
            std::thread::spawn(move || relay_stderr(stderr, &kept));
        }
        Ok(Self {
            child,
            writer: Some(writer),
            lines,
            owed: 0,
            complaint,
            peer,
        })
    }

    /// The last thing the helper said, allowing for it not having arrived yet.
    ///
    /// Standard output and standard error are separate pipes read by separate
    /// threads, so a helper that complains and exits can close the first while
    /// the second is still being drained. A short wait on the error path only
    /// is the difference between naming the reason and reporting silence.
    fn last_complaint(&self) -> Option<String> {
        let deadline = std::time::Instant::now() + Duration::from_millis(200);
        loop {
            if let Ok(complaint) = self.complaint.lock()
                && let Some(said) = complaint.clone()
            {
                return Some(said);
            }
            if std::time::Instant::now() >= deadline {
                return None;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// Where the helper is, given the running program's own location.
    ///
    /// Beside mantaray is where an installation puts it; the debug and release
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
                    // A helper that refuses to start says why on its standard
                    // error and nowhere else; without that, the operator is
                    // told only that something stopped.
                    return Err(failed(match self.last_complaint() {
                        Some(said) => format!("the bridge stopped: {said}"),
                        None => "the bridge stopped".into(),
                    }));
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

/// One detector's lane through a shared bridge - the hub's client half.
///
/// Every lane holds the same [`BridgeTransport`] behind one mutex, and an
/// exchange holds the lock for its whole round trip: the command goes out
/// with the lane's routing mark (`@<n> `), and the reply read back under the
/// same lock belongs to it and nobody else. Lanes can be handed to couriers
/// on their own threads; the mutex is where their transactions take turns,
/// which on the far side is the hub answering one detector at a time.
///
/// Generic over the transport so the routing can be proven against a
/// scripted one; the application uses `HubTransport<BridgeTransport>`.
pub struct HubTransport<T: Transport> {
    line: std::sync::Arc<std::sync::Mutex<T>>,
    detector: i32,
    peer: String,
}

impl<T: Transport> HubTransport<T> {
    /// A lane to `detector` through the shared line.
    pub fn new(line: std::sync::Arc<std::sync::Mutex<T>>, detector: i32) -> Self {
        Self {
            line,
            detector,
            peer: format!("detector {detector} through the hub"),
        }
    }
}

impl<T: Transport> Transport for HubTransport<T> {
    fn exchange(&mut self, command: &str) -> Result<String, DeviceError> {
        let mut line = self
            .line
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        line.exchange(&format!("@{} {command}", self.detector))
    }

    fn peer(&self) -> String {
        self.peer.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::MockTransport;
    use std::sync::{Arc, Mutex};

    /// A lane marks every command with its own detector and no other's.
    #[test]
    fn a_hub_lane_routes_its_own_commands() {
        let shared = Arc::new(Mutex::new(MockTransport::scripted(&[
            ("@3 SHOW_STATUS", "RT=1.00 LT=1.00"),
            ("@7 SHOW_STATUS", "RT=2.00 LT=2.00"),
            ("@3 START", "OK"),
        ])));
        let mut third = HubTransport::new(Arc::clone(&shared), 3);
        let mut seventh = HubTransport::new(Arc::clone(&shared), 7);
        assert_eq!(third.exchange("SHOW_STATUS").unwrap(), "RT=1.00 LT=1.00");
        assert_eq!(seventh.exchange("SHOW_STATUS").unwrap(), "RT=2.00 LT=2.00");
        assert_eq!(third.exchange("START").unwrap(), "OK");
        assert_eq!(third.peer(), "detector 3 through the hub");
    }
}
