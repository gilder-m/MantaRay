//! A thread that holds the line to a slow instrument.
//!
//! [`RemoteMcb`](crate::RemoteMcb) used to speak to the instrument from
//! whatever thread called `poll` - in the application, the thread that draws
//! the frames - and the regular fetch of status and a whole spectrum held
//! that thread for as long as the instrument took. Measured on a bench
//! machine: 325 ms frames, twice a second, on hardware whose ordinary frame
//! costs one millisecond. The interface was not slow; it was waiting.
//!
//! The courier owns the transport instead. Commands still travel one at a
//! time, in the order they were given, and still wait for their answers -
//! Start is something somebody just asked for, and its answer is the point.
//! Only the fetch is different: the caller asks for one and goes back to its
//! frame, and a later poll collects the result from the slot the courier
//! leaves it in. Nothing on the calling thread ever stands waiting on the
//! wire.

use std::sync::mpsc;
use std::sync::{Arc, Mutex};

use crate::error::DeviceError;
use crate::transport::Transport;

/// The status and data lines, fetched together and left unparsed.
///
/// Unparsed deliberately: the courier moves bytes, and every interpretation
/// of them stays in [`RemoteMcb`](crate::RemoteMcb), where it always was.
pub(crate) struct Fetched {
    pub status: String,
    pub data: String,
}

/// What the caller can send the courier to do.
enum Errand {
    /// One command, answered back on the channel the caller waits on.
    Exchange(String, mpsc::Sender<Result<String, DeviceError>>),
    /// Fetch status and data; the outcome goes in the slot for a later poll.
    Fetch,
}

/// The caller's end of the line.
///
/// Dropping it drops the only sender, which is how the courier's thread
/// learns it is finished and lets the transport go.
pub(crate) struct Courier {
    errands: mpsc::Sender<Errand>,
    /// Where the courier leaves a finished fetch.
    slot: Arc<Mutex<Option<Result<Fetched, DeviceError>>>>,
    /// Whether a fetch has been asked for and not yet collected. Kept here,
    /// on the calling side, so asking twice is impossible rather than merely
    /// discouraged - a queue of stale fetches behind a slow instrument would
    /// deliver the past at half-second intervals for minutes.
    fetching: bool,
    peer: String,
}

impl Courier {
    /// Takes the transport onto a thread of its own.
    pub fn carry(transport: Box<dyn Transport>) -> Self {
        let peer = transport.peer();
        let (errands, work) = mpsc::channel();
        let slot = Arc::new(Mutex::new(None));
        let drop_box = Arc::clone(&slot);
        std::thread::Builder::new()
            .name(format!("courier to {peer}"))
            .spawn(move || deliver(transport, &work, &drop_box))
            .expect("spawning the courier thread");
        Self {
            errands,
            slot,
            fetching: false,
            peer,
        }
    }

    /// Sends one command and waits for its answer, as a held transport would.
    ///
    /// If a fetch is mid-flight the wait includes finishing it, which is
    /// bounded by one instrument round trip - no worse than the calling
    /// thread doing the fetch itself, which is what this replaced.
    ///
    /// Whatever that fetch brought is then thrown away. The courier runs
    /// errands in order, so when the answer arrives here, a fetch requested
    /// earlier has already come and gone into the slot - and it describes the
    /// instrument from *before* this command. Collected after a CLEAR, it
    /// would put the thrown-away spectrum straight back, and hand the
    /// start-of-run reconstruction an old clock to plant a date from. A
    /// command is the only thing that makes the slot stale, so this is the
    /// one place that empties it; the next fetch is asked for afresh.
    pub fn exchange(&mut self, command: &str) -> Result<String, DeviceError> {
        let (reply, answer) = mpsc::channel();
        let outcome = self
            .errands
            .send(Errand::Exchange(command.to_string(), reply))
            .map_err(|_| self.gone())
            .and_then(|()| answer.recv().map_err(|_| self.gone()))?;
        self.slot
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();
        self.fetching = false;
        outcome
    }

    /// Asks for a fetch, unless one is already on its way. Never waits.
    pub fn request_fetch(&mut self) {
        if !self.fetching && self.errands.send(Errand::Fetch).is_ok() {
            self.fetching = true;
        }
    }

    /// The finished fetch, if the courier has come back with one.
    ///
    /// A courier whose thread has died - a panic in the transport is the only
    /// way, short of dropping this handle - is reported here as a lost
    /// connection, on every collect. Without that, a death mid-fetch would
    /// leave `fetching` raised forever: no error, no data, a mirror frozen on
    /// numbers that will never change, which reads as a healthy idle
    /// instrument. The thread holds the only other handle to the slot, so its
    /// death is legible from here.
    pub fn collect(&mut self) -> Option<Result<Fetched, DeviceError>> {
        let brought = self
            .slot
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();
        if brought.is_some() {
            self.fetching = false;
            return brought;
        }
        (Arc::strong_count(&self.slot) == 1).then(|| Err(self.gone()))
    }

    /// The error for a courier whose thread has ended: the transport it held
    /// is gone with it, so this is a lost connection, not a protocol fault.
    fn gone(&self) -> DeviceError {
        DeviceError::Connection {
            address: self.peer.clone(),
            detail: "the connection's thread has ended".into(),
        }
    }
}

/// The courier's rounds. Runs until the last sender is dropped.
fn deliver(
    mut transport: Box<dyn Transport>,
    work: &mpsc::Receiver<Errand>,
    slot: &Mutex<Option<Result<Fetched, DeviceError>>>,
) {
    while let Ok(errand) = work.recv() {
        match errand {
            Errand::Exchange(command, reply) => {
                // A caller that stopped waiting for its answer is not an
                // error the courier can do anything about.
                let _ = reply.send(transport.exchange(&command));
            }
            Errand::Fetch => {
                let fetched = transport.exchange("SHOW_STATUS").and_then(|status| {
                    transport
                        .exchange("SHOW_DATA")
                        .map(|data| Fetched { status, data })
                });
                *slot.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(fetched);
            }
        }
    }
}
