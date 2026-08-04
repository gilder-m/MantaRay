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

use crate::error::DeviceError;

/// A byte pipe that carries one command out and one response line back.
pub trait Transport: Send {
    /// Sends one command and returns the response line, without its newline.
    fn exchange(&mut self, command: &str) -> Result<String, DeviceError>;

    /// Where this leads, for messages ("192.168.0.40:2000", "mock").
    fn peer(&self) -> String;
}

/// The dialect over TCP: commands end CRLF, responses end LF.
pub struct TcpTransport {
    reader: BufReader<TcpStream>,
    writer: TcpStream,
    peer: String,
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
        let first = addresses.first().ok_or_else(|| DeviceError::Connection {
            address: address.to_string(),
            detail: "the address resolves to nothing".into(),
        })?;
        let stream = TcpStream::connect_timeout(first, timeout).map_err(|error| {
            DeviceError::Connection {
                address: address.to_string(),
                detail: error.to_string(),
            }
        })?;
        stream.set_read_timeout(Some(timeout)).ok();
        stream.set_write_timeout(Some(timeout)).ok();
        stream.set_nodelay(true).ok();
        let writer = stream
            .try_clone()
            .map_err(|error| DeviceError::Connection {
                address: address.to_string(),
                detail: error.to_string(),
            })?;
        Ok(Self {
            reader: BufReader::new(stream),
            writer,
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
        self.writer
            .write_all(command.as_bytes())
            .and_then(|_| self.writer.write_all(b"\r\n"))
            .map_err(|error| failed(error.to_string()))?;
        let mut line = String::new();
        let read = self
            .reader
            .read_line(&mut line)
            .map_err(|error| failed(error.to_string()))?;
        if read == 0 {
            return Err(failed("the instrument closed the connection".into()));
        }
        Ok(line.trim_end_matches(['\r', '\n']).to_string())
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
