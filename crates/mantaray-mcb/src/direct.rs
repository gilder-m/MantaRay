//! The DPM-USB adapter over libusb, with no vendor driver at all.
//!
//! On Windows the adapter belongs to ORTEC's driver and is reached through its
//! IOCTLs; see `usb.rs`, which exists only there. Everywhere else there is no
//! driver to install:
//! the kernel's own USB stack hands the interface over, and the same frames go
//! down the same two bulk endpoints. That is the whole difference, which is why
//! [`crate::dpm`] sits on a trait rather than on either one.
//!
//! What this does **not** do is take a device away from another driver. On
//! Windows ORTEC's already owns it, so this is not wired in there; on Linux and
//! macOS nothing owns it and there is nothing to take.

use std::cell::RefCell;
use std::time::Duration;

use nusb::MaybeFuture;
use nusb::transfer::{Bulk, In, Out};

use crate::dpm::BulkDevice;

/// The adapter, as the bus reports it.
pub const VENDOR: u16 = 0x0A2D;
/// The DPM-USB product.
pub const PRODUCT: u16 = 0x0016;

/// The interface the bulk endpoints live on.
const INTERFACE: u8 = 0;
/// Frames to the adapter.
const OUT: u8 = 0x01;
/// Answers back.
const IN: u8 = 0x81;

/// An adapter reached through libusb.
///
/// The endpoints are opened once and kept, because claiming them per transfer
/// would be both slow and a good way to lose a queued answer. They sit behind
/// a `RefCell` so that sending stays a `&self` operation, which is what the
/// protocol layer above expects.
pub struct Device {
    out: RefCell<nusb::Endpoint<Bulk, Out>>,
    input: RefCell<nusb::Endpoint<Bulk, In>>,
    /// Kept for [`Device::cycle`], which is the only thing that needs the
    /// device itself rather than one of its endpoints.
    device: nusb::Device,
    serial: String,
}

/// How long to wait on a leftover answer before deciding there is not one.
///
/// An answer already sitting in the adapter arrives at once, so this only has
/// to outlast the bus, not the instrument.
const DRAIN_TIMEOUT: Duration = Duration::from_millis(50);

/// Reads away answers left over from whoever held the adapter last.
///
/// Bounded rather than "until it goes quiet": an adapter that talks without
/// stopping would otherwise keep the program in here for good, and a handful
/// of frames is already far more than the one stale reply this is for.
fn drain(input: &mut nusb::Endpoint<Bulk, In>) {
    let packet = input.max_packet_size().max(1);
    for _ in 0..8 {
        let buffer = input.allocate(packet);
        let done = input.transfer_blocking(buffer, DRAIN_TIMEOUT);
        // Timing out is the expected end of this: nothing was waiting.
        if done.status.is_err() || done.actual_len == 0 {
            return;
        }
    }
}

/// Every DPM-USB adapter the bus can see.
fn adapters() -> Result<Vec<nusb::DeviceInfo>, String> {
    Ok(nusb::list_devices()
        .wait()
        .map_err(|error| format!("listing USB devices: {error}"))?
        .filter(|device| device.vendor_id() == VENDOR && device.product_id() == PRODUCT)
        .collect())
}

impl Device {
    /// The serial number of every adapter present.
    pub fn list() -> Result<Vec<String>, String> {
        Ok(adapters()?
            .iter()
            .map(|device| {
                device
                    .serial_number()
                    .map(str::to_string)
                    .unwrap_or_else(|| format!("{}:{}", device.bus_id(), device.device_address()))
            })
            .collect())
    }

    /// Opens an adapter: the one whose serial contains `wanted`, or the first
    /// one there when nothing is asked for.
    pub fn open(wanted: Option<&str>) -> Result<Self, String> {
        let found = adapters()?;
        if found.is_empty() {
            return Err(format!(
                "no DPM-USB adapter on the bus (looking for {VENDOR:04x}:{PRODUCT:04x})"
            ));
        }
        let chosen = match wanted {
            None => &found[0],
            Some(wanted) => found
                .iter()
                .find(|device| {
                    device
                        .serial_number()
                        .is_some_and(|serial| serial.contains(wanted))
                })
                .ok_or_else(|| format!("no adapter with {wanted:?} in its serial number"))?,
        };
        let serial = chosen
            .serial_number()
            .map(str::to_string)
            .unwrap_or_default();
        let device = chosen
            .open()
            .wait()
            .map_err(|error| format!("opening adapter {serial}: {error}"))?;
        let interface = device.claim_interface(INTERFACE).wait().map_err(|error| {
            // Two different faults arrive here and they need opposite advice.
            // Busy means something already holds the interface - another
            // window of this program, or a kernel driver that bound it - and
            // no udev rule will help. Denied is the permission case. Sending
            // somebody to write a udev rule when the real answer is "you have
            // it open over there" is the more likely of the two now that a
            // bench can carry more than one adapter.
            let advice = match error.kind() {
                nusb::ErrorKind::Busy => {
                    "Something else already has this adapter - another window of this \
                     program, or a driver that has bound it. Close the other one, or \
                     pick a different adapter."
                        .to_string()
                }
                nusb::ErrorKind::PermissionDenied => format!(
                    "This is permission: a udev rule giving {VENDOR:04x}:{PRODUCT:04x} \
                     to your user is what is wanted."
                ),
                nusb::ErrorKind::Disconnected => {
                    "The adapter went away between being listed and being opened.".to_string()
                }
                _ => "The adapter answered, but the interface could not be claimed.".to_string(),
            };
            format!("claiming interface {INTERFACE} of adapter {serial}: {error}. {advice}")
        })?;
        let out = interface
            .endpoint::<Bulk, Out>(OUT)
            .map_err(|error| format!("opening endpoint {OUT:#04x}: {error}"))?;
        let input = interface
            .endpoint::<Bulk, In>(IN)
            .map_err(|error| format!("opening endpoint {IN:#04x}: {error}"))?;
        // Deliberately *not* settled here. Draining an endpoint that has
        // nothing to say costs a run of cancelled reads, and on this transport
        // that is not free: the instrument answers an IN token it is never
        // given credit for, advances its data toggle, and from then on speaks
        // past a host whose own toggle never moved. Measured on a 926, opening
        // like that turned an adapter that answers into one that never answers
        // again. Settling is a repair, so it is asked for - see `usbfix` - and
        // not done to a healthy adapter on the way past.
        Ok(Self {
            out: RefCell::new(out),
            input: RefCell::new(input),
            device,
            serial,
        })
    }

    /// The adapter's serial number, which is the instrument's identity here.
    pub fn serial(&self) -> &str {
        &self.serial
    }

    /// Brings both bulk endpoints back to a known state.
    ///
    /// The counterpart of the Windows `settle`, for the fault it is named
    /// after: the adapter answers what it was last asked whether or not anyone
    /// is still listening, so a request abandoned by some earlier program
    /// leaves a reply queued, and every answer after it is one question late.
    /// Reading until nothing more comes back is what puts the two back in step.
    ///
    /// Unlike the Windows one this is **not** run on opening or after a
    /// transfer times out. Over libusb the reads it costs do their own harm -
    /// see the note in [`Device::open`] - so it is worth doing only once an
    /// adapter is known to be stuck, which is [`crate::usb_fix`]'s job.
    /// The order matters and is the opposite of the obvious one. Reading the
    /// queue empty is what puts question and answer back in step, but every
    /// read that finds nothing is a cancelled transfer, and those are what put
    /// the two ends' data toggles out of step in the first place. So the halts
    /// are cleared last, once there is nothing left to read: clearing a halt
    /// resets the toggle, and doing it first only means throwing away the
    /// repair before the damage.
    pub fn settle(&self) {
        {
            let mut input = self.input.borrow_mut();
            drain(&mut input);
        }
        self.out.borrow_mut().clear_halt().wait().ok();
        self.input.borrow_mut().clear_halt().wait().ok();
    }

    /// Unplugs and replugs the adapter, in software.
    ///
    /// For the state a pipe reset cannot reach: the adapter's own state machine
    /// stuck, refusing even to take a frame. It re-enumerates, so this handle
    /// is finished afterwards - hence taking `self` - and the caller opens the
    /// adapter afresh.
    pub fn cycle(self) -> Result<(), String> {
        let Self {
            out,
            input,
            device,
            serial,
        } = self;
        // A device with a claimed interface will not reset, and the endpoints
        // are what hold the claim, so they have to go first. This is the reason
        // the whole handle is consumed rather than borrowed.
        drop(out);
        drop(input);
        device
            .reset()
            .wait()
            .map_err(|error| format!("cycling adapter {serial}: {error}"))
    }
}

impl BulkDevice for Device {
    // Every transfer goes through `transfer_blocking`, which cancels on
    // timeout and does not return until the transfer has actually come back.
    // The obvious `submit` + `wait_next_complete` pair is a trap: a timed-out
    // transfer stays queued, its completion is handed to the *next* request as
    // if it were the answer, and from then on every reply is one question
    // late. nusb's own documentation warns of exactly this.
    fn bulk(&self, endpoint: u8, data: &mut [u8], milliseconds: u32) -> Result<usize, String> {
        let timeout = Duration::from_millis(u64::from(milliseconds));
        use nusb::transfer::TransferError;
        // The top bit of an endpoint address is its direction, which decides
        // whether these bytes are being sent or are about to be filled in.
        // A failure here is deliberately left where it lies rather than settled
        // on the spot. A timed-out read is the ordinary way this instrument
        // says "not yet" - the first question after an adapter is opened goes
        // unanswered every time on a 926 - and settling in response would treat
        // the common case as damage, at the price of the cancelled reads that
        // cause the real thing. Recovery belongs to `usbfix`, where somebody
        // has decided the adapter is actually stuck.
        if endpoint & 0x80 == 0 {
            let mut out = self.out.borrow_mut();
            let done = out.transfer_blocking(data.to_vec().into(), timeout);
            match done.status {
                Ok(()) => Ok(done.actual_len),
                Err(TransferError::Cancelled) => Err(format!(
                    "the adapter did not accept a frame within {milliseconds} ms"
                )),
                Err(error) => Err(format!("writing to endpoint {endpoint:#04x}: {error}")),
            }
        } else {
            self.read(data, timeout, milliseconds)
        }
    }
}

impl Device {
    /// The reading half of [`BulkDevice::bulk`], split out so that the borrow
    /// of the endpoint is over before a failure is settled.
    fn read(&self, data: &mut [u8], timeout: Duration, milliseconds: u32) -> Result<usize, String> {
        use nusb::transfer::TransferError;
        let mut input = self.input.borrow_mut();
        // A read must be submitted in whole packets; the protocol layer
        // already rounds up, and this keeps the promise explicit.
        let packet = input.max_packet_size().max(1);
        let room = data.len().div_ceil(packet) * packet;
        let buffer = input.allocate(room);
        let done = input.transfer_blocking(buffer, timeout);
        match done.status {
            Ok(()) => {
                let taken = done.actual_len.min(data.len());
                data[..taken].copy_from_slice(&done.buffer[..taken]);
                Ok(taken)
            }
            Err(TransferError::Cancelled) => Err(format!(
                "the instrument did not answer within {milliseconds} ms"
            )),
            Err(error) => Err(format!("reading from endpoint {IN:#04x}: {error}")),
        }
    }
}
