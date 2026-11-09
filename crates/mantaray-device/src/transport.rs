//! How words reach an instrument: a command line out, a response line back.
//!
//! Real ORTEC hardware is reached several ways - Ethernet, USB through the
//! vendor's CONNECTIONS layer, serial adapters - but every one of them carries
//! the same ASCII dotted-command dialect (`START`, `SET_PRESET_LIVE 300`,
//! `SHOW_STATUS`). [`Transport`] is that seam: [`RemoteMcb`](crate::RemoteMcb)
//! speaks the dialect and does not care what carries it. TCP is built in;
//! a vendor-library transport can be added without touching the protocol; and
//! tests use a scripted transport, so the protocol layer is proven before any
//! hardware is on the bench.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::time::Duration;

#[cfg(feature = "rig")]
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use crate::error::DeviceError;
#[cfg(feature = "rig")]
use crate::remote::RemoteMcb;

/// A byte pipe that carries one command out and one response line back.
pub trait Transport: Send {
    /// Sends one command and returns the response line, without its newline.
    fn exchange(&mut self, command: &str) -> Result<String, DeviceError>;

    /// Where this leads, for messages ("192.168.0.40:2000", "mock").
    fn peer(&self) -> String;
}

/// The dialect over TCP: commands end CRLF, responses end LF.
///
/// A reply that misses the timeout is not lost: it arrives later. On a
/// connection that simply carried on, the next command read it as its own
/// answer and every command after read its predecessor's, so one late reply
/// put a network detector out of step until it was removed and added again
/// (deep review F3). So a failed or timed-out exchange ends the connection
/// and the next exchange opens a fresh one: a late reply dies with the old
/// socket and is never read as another command's answer.
pub struct TcpTransport {
    /// The open connection (reader, writer); `None` after a failed exchange.
    line: Option<(BufReader<TcpStream>, TcpStream)>,
    address: std::net::SocketAddr,
    timeout: Duration,
    peer: String,
}

/// One connection, with the timeout on its reads and writes.
fn open(
    address: std::net::SocketAddr,
    timeout: Duration,
) -> std::io::Result<(BufReader<TcpStream>, TcpStream)> {
    let stream = TcpStream::connect_timeout(&address, timeout)?;
    stream.set_read_timeout(Some(timeout)).ok();
    stream.set_write_timeout(Some(timeout)).ok();
    stream.set_nodelay(true).ok();
    let writer = stream.try_clone()?;
    Ok((BufReader::new(stream), writer))
}

impl TcpTransport {
    /// Connects, with a timeout on everything so a silent instrument fails
    /// with a message rather than hanging the caller.
    pub fn connect(address: &str, timeout: Duration) -> Result<Self, DeviceError> {
        let addresses: Vec<std::net::SocketAddr> =
            std::net::ToSocketAddrs::to_socket_addrs(address)
                .map_err(|error| DeviceError::Connection {
                    address: address.to_string(),
                    detail: error.to_string(),
                })?
                .collect();
        let first = *addresses.first().ok_or_else(|| DeviceError::Connection {
            address: address.to_string(),
            detail: "the address resolves to nothing".into(),
        })?;
        let line = open(first, timeout).map_err(|error| DeviceError::Connection {
            address: address.to_string(),
            detail: error.to_string(),
        })?;
        Ok(Self {
            line: Some(line),
            address: first,
            timeout,
            peer: address.to_string(),
        })
    }
}

impl Transport for TcpTransport {
    fn exchange(&mut self, command: &str) -> Result<String, DeviceError> {
        let failed = |detail: String| DeviceError::Connection {
            address: self.peer.clone(),
            detail,
        };
        if self.line.is_none() {
            let line = open(self.address, self.timeout).map_err(|error| {
                failed(format!("reconnecting after a failed exchange: {error}"))
            })?;
            self.line = Some(line);
        }
        let Some((reader, writer)) = self.line.as_mut() else {
            return Err(failed("not connected".into()));
        };
        let mut line = String::new();
        let outcome = writer
            .write_all(command.as_bytes())
            .and_then(|()| writer.write_all(b"\r\n"))
            .and_then(|()| reader.read_line(&mut line));
        match outcome {
            Ok(0) => {
                self.line = None;
                Err(failed("the instrument closed the connection".into()))
            }
            Ok(_) => Ok(line.trim_end_matches(['\r', '\n']).to_string()),
            Err(error) => {
                // Whatever is still owed on this connection - the late reply,
                // or the rest of a half-read line - goes with it.
                if let Some((_, writer)) = self.line.take() {
                    let _ = writer.shutdown(std::net::Shutdown::Both);
                }
                Err(failed(error.to_string()))
            }
        }
    }

    fn peer(&self) -> String {
        self.peer.clone()
    }
}

/// A scripted transport for tests: expected commands in, canned responses out.
pub struct MockTransport {
    /// Pairs of (expected command, response). Consumed in order.
    script: std::collections::VecDeque<(String, String)>,
    /// Everything actually sent, for assertions.
    pub sent: Vec<String>,
    /// A response used when the script is empty ("OK" by default).
    pub fallback: String,
}

impl MockTransport {
    /// A transport that answers everything with the fallback.
    pub fn lenient() -> Self {
        Self {
            script: std::collections::VecDeque::new(),
            sent: Vec::new(),
            fallback: "OK".into(),
        }
    }

    /// A transport that checks each command against a script.
    pub fn scripted(pairs: &[(&str, &str)]) -> Self {
        Self {
            script: pairs
                .iter()
                .map(|(command, response)| (command.to_string(), response.to_string()))
                .collect(),
            sent: Vec::new(),
            fallback: "OK".into(),
        }
    }
}

impl Transport for MockTransport {
    fn exchange(&mut self, command: &str) -> Result<String, DeviceError> {
        self.sent.push(command.to_string());
        match self.script.pop_front() {
            Some((expected, response)) => {
                if command != expected {
                    return Err(DeviceError::Command {
                        command: command.to_string(),
                        detail: format!("the script expected {expected:?}"),
                    });
                }
                Ok(response)
            }
            None => Ok(self.fallback.clone()),
        }
    }

    fn peer(&self) -> String {
        "mock".into()
    }
}

/// A line to a quiet eight-channel instrument that logs every command sent,
/// for tests that care what did *not* go out.
///
/// Behind the `rig` feature, which only test builds turn on: its answers are
/// invented, so no shipped binary may carry it. Unlike [`MockTransport`],
/// whose log is lost once the transport is boxed into a
/// [`RemoteMcb`](crate::RemoteMcb), the log here is shared, so a test (a GUI
/// test included) can read it while the instrument is in use.
#[cfg(feature = "rig")]
pub struct RecordingTransport {
    log: CommandLog,
}

/// Every command a [`RecordingTransport`] was sent, oldest first.
#[cfg(feature = "rig")]
pub type CommandLog = Arc<Mutex<Vec<String>>>;

#[cfg(feature = "rig")]
impl RecordingTransport {
    /// The line and a handle on its log.
    #[must_use]
    pub fn new() -> (Self, CommandLog) {
        let log = CommandLog::default();
        (
            Self {
                log: Arc::clone(&log),
            },
            log,
        )
    }

    /// A remote instrument on a recording line, with the handshake cleared
    /// from the log so it holds only what the caller causes.
    ///
    /// # Errors
    ///
    /// Whatever [`RemoteMcb::connect`] refuses; the canned handshake itself
    /// is always accepted.
    pub fn connect(number: u16, name: &str) -> Result<(RemoteMcb, CommandLog), DeviceError> {
        let (line, log) = Self::new();
        let remote = RemoteMcb::connect(Box::new(line), number, name)?;
        lock(&log).clear();
        Ok((remote, log))
    }

    /// The commands in the log that change the instrument, in order: all but
    /// the `SHOW_` reads. A command not known here is kept, not hidden.
    pub fn mutating(log: &Mutex<Vec<String>>) -> Vec<String> {
        lock(log)
            .iter()
            .filter(|command| !command.starts_with("SHOW_"))
            .cloned()
            .collect()
    }
}

/// The log, even after a test panicked holding it: what was sent is still
/// what was sent.
#[cfg(feature = "rig")]
fn lock(log: &Mutex<Vec<String>>) -> MutexGuard<'_, Vec<String>> {
    log.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(feature = "rig")]
impl Transport for RecordingTransport {
    fn exchange(&mut self, command: &str) -> Result<String, DeviceError> {
        lock(&self.log).push(command.to_string());
        Ok(match command {
            "SHOW_CONFIGURATION" => "MODEL=DSPEC-50 SERIAL=1 FIRMWARE=1 CHANNELS=8".into(),
            "SHOW_STATUS" => "RT=0.00 LT=0.00 DT=0.0% ICR=0 ACTIVE=0 TOTAL=0".into(),
            "SHOW_DATA" => "DATA 8 0 0 0 0 0 0 0 0".into(),
            "SHOW_PRESETS" => "PRESETS REAL=0.00 LIVE=300.00 PEAK=0 INTEG=0".into(),
            _ => "OK".into(),
        })
    }

    fn peer(&self) -> String {
        "recording".into()
    }
}
