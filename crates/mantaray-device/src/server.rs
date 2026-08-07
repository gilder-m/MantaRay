//! The other end of the wire: any [`Mcb`] served over the dialect.
//!
//! This is what makes the remote layer honest - and useful before hardware
//! arrives. `serve` puts an instrument (usually a
//! [`SimulatedMcb`](crate::SimulatedMcb)) behind a
//! TCP listener speaking exactly the dialect [`RemoteMcb`](crate::RemoteMcb)
//! expects, so one mantaray can hand its simulator to another across the room,
//! and the protocol tests run against the same code a real bench setup would.

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};

use crate::mcb::Mcb;

/// Serves one client connection to completion.
///
/// Reads a command a line at a time and answers each on one line; `ERR reason`
/// carries a rejection. Returns when the client hangs up.
pub fn serve_connection(stream: TcpStream, mcb: Arc<Mutex<dyn Mcb>>) -> std::io::Result<()> {
    // One silent client must not hold the single serving slot forever: the
    // other end polls every half second when it is alive at all, so a minute
    // of nothing is a vanished client, not a quiet one.
    stream.set_read_timeout(Some(std::time::Duration::from_secs(60)))?;
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut writer = stream;
    let mut line = String::new();
    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            return Ok(());
        }
        let command = line.trim();
        if command.is_empty() {
            continue;
        }
        let response = {
            // A poisoned lock means the clock thread panicked mid-advance;
            // the simulator's state is still the best answer there is, and
            // killing the serve loop over it would refuse every later client.
            let mut mcb = mcb.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            mcb.send_message(command)
                .unwrap_or_else(|error| format!("ERR {error}"))
        };
        writer.write_all(response.as_bytes())?;
        writer.write_all(b"\n")?;
    }
}

/// Accepts clients forever, one at a time.
///
/// The instrument is shared behind a mutex, so the owner can keep advancing it
/// (the simulator's clock, for instance) between commands.
pub fn serve(listener: TcpListener, mcb: Arc<Mutex<dyn Mcb>>) -> std::io::Result<()> {
    for stream in listener.incoming() {
        let stream = stream?;
        // One client at a time: an MCB is a single-operator instrument.
        let _ = serve_connection(stream, Arc::clone(&mcb));
    }
    Ok(())
}
